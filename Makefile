db-run:
	docker-compose -f ./db/docker-compose.yml up -d
db-stop:
	docker stop twirc_db
db-migrate:
	./vendor/bin/refinery migrate -c ./db/refinery.toml -p ./db/migrations
build:
	cargo build
app-run:
	RUST_LOG=info ./target/debug/twirc -s reihatori
vendor:
	@if [ ! -f vendor/bin/refinery ]; then \
		mkdir -p vendor && \
		cargo install --root vendor refinery_cli; \
	fi
