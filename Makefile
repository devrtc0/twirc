PATH := vendor/bin:$(PATH)

db-run:
	docker-compose -f ./db/docker-compose.yml up -d

db-clean:
	docker-compose -f ./db/docker-compose.yml down --remove-orphans

db-stop:
	docker stop twirc_db twirc_scylla_db

db-migrate:
	refinery migrate -c ./db/refinery.toml -p ./db/migrations

app-run:
	RUST_LOG=info cargo run -- -s mudriy_ork

vendor:
	@if [ ! -f vendor/bin/refinery ]; then \
		mkdir -p vendor && \
		cargo install --root vendor refinery_cli; \
	fi
