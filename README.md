# Micro challenger

Local evaluation platform for the stream-processing challenge. It serves an input
event stream over a REST API, receives the answers, and reports correctness and
timings, overall and per track.

It supports starting benchmark runs, fetching input batches, submitting results and
recording the timings. The `/dash` endpoint is used to browse previous runs.

## Data format

Input CSV (`event_time,request_id,request_type,params,tx_id,spends,creates`):
- events with an empty `request_id` are transactions: they only update state and need no answer;
- events with a numeric `request_id` are requests: each one needs an answer.

Output CSV (`event_time,request_id,request_type,result`) holds the expected answer
for every request.

## Loading the data

The challenger uses an embedded database, stored in a local directory (`data/db` below).
No external database server is needed. The database must be built once from the two CSV
files, and is then reused by every run.

1. Build the database. The bind address is required by the CLI but unused in this
   step, and the process exits when loading ends:

   ```
   micro-challenger --load-csv-input input.csv --load-csv-output output.csv \
       --max-batch-size 1024 127.0.0.1:0 data/db
   ```

2. Start the server on it:

   ```
   micro-challenger 127.0.0.1:8866 data/db
   ```

Loading again into an existing directory adds to its content: remove the directory to
start from scratch.

- Consecutive transactions are grouped in batches of at most `--max-batch-size`
  lines. Each request is always a batch of its own.
- A batch is returned as several newline-separated CSV lines. Clients have to split
  it and process every line.

### Tracks

Requests can be split in tracks. Load a JSON map from `request_type` to track name
(for example `{"BALANCE": "basic", "TOPK": "ranking"}`):

```
micro-challenger --load-request-mapping tracks.json 127.0.0.1:0 data/db
```

It can be combined with the CSV load. Requests whose type is not in the map are
included only in the overall statistics.

## REST API

All endpoints are under `/api`.

| Endpoint | Description |
|---|---|
| `POST /create` | Create a benchmark, returns its id. Body: `{"apitoken", "test", "name"?, "max_batches"?}` |
| `POST /start/{bench-id}` | Start the benchmark |
| `GET /next_batch/{bench-id}` | Next batch of input lines; `404` when the input is finished |
| `POST /result/{bench-id}/{request-id}` | Submit the answer (an output CSV line) for a request |
| `POST /end/{bench-id}` | Finish the run and get the report |
| `GET /history` | Summary of previous runs, one entry per track |

`/end` returns `{"overall": {...}, "tracks": {"<track>": {...}}}`. A track appears
only if at least one of its requests was answered. Each stats object has:

- `count`: number of requests (not events), answered or not
- `missing`: requests without an answer, including requests never fetched
- `correct` / `wrong`: answers equal to / different from the expected output
- `throughput`: answered requests per second, from the first input batch sent until the last answer received
- `latency_mean`, `latency_min`, `latency_max`, `latency_p99`: over answered requests,
  from the moment the request is handed out until its answer arrives

`/api/end/{bench-id}` must be called for the results to be saved and shown in `/dash`.

The examples in `examples/` (`democlient`, `democlient-par`) show a complete client.

## Other options

- `--list-results`, `--csv-results`: print stored runs
- `--delete-result <bench-id>`: remove a stored run

`just run`, `just demo` and `just all` (see `Justfile`) build and run the server
and a demo client.
