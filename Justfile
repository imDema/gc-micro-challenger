default: all

PROFILE := "release"
TARGET_DIR := if PROFILE == "dev" { "debug" } else { PROFILE }

# Load CSV data into a fresh sled database
load:
	cargo build --profile {{PROFILE}}
	rm -rf data/db
	./target/{{TARGET_DIR}}/micro-challenger \
		--load-csv-input data/input.csv --load-csv-output data/output.csv \
		--load-request-mapping tracks.json \
		127.0.0.1:0 data/db

# Load CSV and start the server (background)
run:
	cargo build --profile {{PROFILE}} --bin micro-challenger
	./target/{{TARGET_DIR}}/micro-challenger --max-batch-size 1024 127.0.0.1:18080 data/db

# Run the example demo client against the running server
demo:
	cargo build --profile {{PROFILE}} --example democlient
	./target/{{TARGET_DIR}}/examples/democlient http://127.0.0.1:18080

demo-par:
	cargo build --profile {{PROFILE}} --example democlient-par
	./target/{{TARGET_DIR}}/examples/democlient-par http://127.0.0.1:18080

# Full pipeline: load, start server, run demo client
all:
	cargo build --profile {{PROFILE}}
	rm -rf data/db
	./target/{{TARGET_DIR}}/micro-challenger --load-csv-input data/input.csv --load-csv-output data/output.csv --max-batch-size 1024 127.0.0.1:18080 data/db
	./target/{{TARGET_DIR}}/micro-challenger 127.0.0.1:18080 data/db &
	@sleep 2
	./target/{{TARGET_DIR}}/examples/democlient http://127.0.0.1:18080
	killall micro-challenger

