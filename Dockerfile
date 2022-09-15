from postgres:11
COPY migrations/init.sql docker-entrypoint-initdb.d/
