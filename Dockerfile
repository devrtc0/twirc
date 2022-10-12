from postgres:14
COPY migrations/init.sql docker-entrypoint-initdb.d/
