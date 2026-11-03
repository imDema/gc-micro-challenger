use parking_lot::RwLock;

use crossbeam::utils::CachePadded;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct BatchResult {
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
    pub data: Box<[u8]>,
}

impl std::fmt::Debug for BatchResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryResult")
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
    Completed(BatchResult),
}

impl LogCell {
    fn as_completed(&self) -> Option<&BatchResult> {
        if let Self::Completed(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

type InnerLog = Vec<LogCell>;

pub struct ResultLog {
    shards: Box<[CachePadded<RwLock<InnerLog>>]>,
}

impl ResultLog {
    pub fn new(capacity: usize, shards: usize) -> Self {
        let shards = shards.next_power_of_two();
        let shards = (0..shards)
            .map(|_| {
                CachePadded::new(RwLock::new(InnerLog::with_capacity(
                    capacity.div_ceil(shards),
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

    pub fn put(&self, index: usize, data: impl Into<Box<[u8]>>) -> Option<OffsetDateTime> {
        let timestamp = OffsetDateTime::now_utc();
        let shard = index % self.shards.len();
        let inner_index = index / self.shards.len();

        let mut shard = self.shards[shard].write();
        if inner_index >= shard.len() {
            tracing::warn!("query response for uninit batch {index}!!");
            return None;
        }

        match shard[inner_index] {
            LogCell::Empty => tracing::warn!("query response for uninit batch {index}!"),
            LogCell::Completed(..) => {
                tracing::error!("duplicate query response for index {index}!")
            }
            LogCell::Started(start) => {
                shard[inner_index] = LogCell::Completed(BatchResult {
                    start,
                    end: timestamp,
                    data: data.into(),
                });
                return Some(timestamp);
            }
        }
        None
    }

    pub fn start(&self, index: usize) {
        let shard = index % self.shards.len();
        let inner_index = index / self.shards.len();

        let mut shard = self.shards[shard].write();
        if inner_index >= shard.len() {
            shard.resize_with((inner_index + 1).next_power_of_two(), || Default::default());
        }

        match shard[inner_index] {
            LogCell::Empty => shard[inner_index] = LogCell::Started(OffsetDateTime::now_utc()),
            LogCell::Started(..) => tracing::error!("trying to reinit started query {index}!"),
            LogCell::Completed(..) => tracing::error!("trying to reinit completed query {index}!"),
        }
    }

    pub fn to_vec(self) -> Vec<Option<BatchResult>> {
        let mut shards = self
            .shards
            .iter()
            .map(|s| std::mem::take::<Vec<_>>(s.write().as_mut()).into_iter())
            .collect::<Vec<_>>();
        let capacity = shards.iter().map(|s| s.len()).sum::<usize>();

        let mut output = Vec::with_capacity(capacity);
        for index in 0..capacity {
            let shard = index % shards.len();
            output.push(
                shards[shard]
                    .next()
                    .unwrap_or_default()
                    .as_completed()
                    .cloned(),
            );
        }
        output
    }
}
