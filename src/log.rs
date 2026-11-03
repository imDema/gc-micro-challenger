use parking_lot::RwLock;

use crossbeam::utils::CachePadded;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct CompleteResult {
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
    pub data: Box<[u8]>,
}

impl std::fmt::Debug for CompleteResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchResult")
            .field("start", &self.start)
            .field("end", &self.end)
            .field("size", &self.data.len())
            .finish()
    }
}

#[derive(Default, Debug)]
enum LogCell {
    #[default]
    Empty,
    Started(OffsetDateTime),
    Completed(CompleteResult),
}

impl LogCell {
    fn as_result(&self) -> Option<&CompleteResult> {
        if let Self::Completed(v) = self {
            Some(v)
        } else {
            None
        }
    }

    fn is_started(&self) -> bool {
        !matches!(self, LogCell::Empty)
    }
}

type InnerLog = Vec<LogCell>;

pub struct ResultLog {
    shards: Box<[CachePadded<RwLock<InnerLog>>]>,
}

impl ResultLog {
    pub fn new(capacity: usize, shard_count: usize) -> Self {
        let shard_count = shard_count.next_power_of_two();
        let shards = (0..shard_count)
            .map(|_| {
                CachePadded::new(RwLock::new(InnerLog::with_capacity(
                    capacity.div_ceil(shard_count),
                )))
            })
            .collect();

        Self { shards }
    }

    pub fn resize(&self, capacity: usize) {
        let cap = capacity.div_ceil(self.shards.len());
        for s in self.shards.iter() {
            let mut s = s.write();
            s.resize_with(cap, || LogCell::Empty);
        }
    }

    /// Record a completed result with its data. Persisted to SLED at completion.
    pub fn put(&self, index: usize, data: impl Into<Box<[u8]>>) -> Option<OffsetDateTime> {
        let timestamp = OffsetDateTime::now_utc();
        let shard = index % self.shards.len();
        let inner_index = index / self.shards.len();

        let mut shard = self.shards[shard].write();
        if inner_index >= shard.len() {
            tracing::warn!("result for uninit index {index}!!");
            return None;
        }

        match shard[inner_index] {
            LogCell::Empty => tracing::warn!("result for uninit index {index}!"),
            LogCell::Completed(..) => {
                tracing::error!("duplicate result for index {index}!")
            }
            LogCell::Started(start) => {
                shard[inner_index] = LogCell::Completed(CompleteResult {
                    start,
                    end: timestamp,
                    data: data.into(),
                });
                return Some(timestamp);
            }
        }
        None
    }

    /// Mark this index as dispatched. Called when the query is sent.
    pub fn start(&self, index: usize) {
        let shard = index % self.shards.len();
        let inner_index = index / self.shards.len();
        let mut shard = self.shards[shard].write();
        if inner_index >= shard.len() {
            shard.resize_with((inner_index + 1).next_power_of_two(), Default::default);
        }

        match shard[inner_index] {
            LogCell::Empty => shard[inner_index] = LogCell::Started(OffsetDateTime::now_utc()),
            LogCell::Started(..) => tracing::error!("trying to reinit started index {index}!"),
            LogCell::Completed(..) => tracing::error!("trying to reinit completed index {index}!"),
        }
    }

    pub fn count_started(&self) -> usize {
        self.shards
            .iter()
            .map(|s| s.read().iter().filter(|&c| c.is_started()).count())
            .max()
            .unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .enumerate()
            .map(|(shard, s)| {
                s.read()
                    .iter()
                    .enumerate()
                    .rev()
                    .skip_while(|(_, c)| !c.is_started())
                    .next()
                    .map(|(i, _)| (i * self.shards.len() + shard) + 1)
                    .unwrap_or_default()
            })
            .max()
            .unwrap_or_default()
    }

    /// Reassemble results in index order.
    pub fn to_completed_vec(self) -> Vec<Option<CompleteResult>> {
        let capacity = self.len();
        let mut shards = self
            .shards
            .iter()
            .map(|s| std::mem::take::<Vec<_>>(s.write().as_mut()).into_iter())
            .collect::<Vec<_>>();

        let mut output = Vec::with_capacity(capacity);
        for index in 0..capacity {
            let shard = index % shards.len();
            let cell = shards[shard].next().unwrap_or_default();
            output.push(cell.as_result().cloned());
        }
        output
    }
}
