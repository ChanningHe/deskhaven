# Use PostgreSQL

When you finish, deskhaven stores its data in PostgreSQL instead of SQLite.

Set this up before the first start: nothing moves data from SQLite to PostgreSQL.

## Compose

Follow the [quick start](../getting-started/quick-start.md) through step 2. Then create a
password:

```sh
echo "POSTGRES_PASSWORD=$(openssl rand -hex 24)" >> .env
```

Merge this into `compose.override.yaml`
([how](../reference/configuration.md#in-the-compose-deployment)):

```yaml
services:
  deskhaven:
    environment:
      DESKHAVEN_DATABASE__URL: postgres://deskhaven:${POSTGRES_PASSWORD}@postgres/deskhaven
    depends_on:
      postgres:
        condition: service_healthy
  postgres:
    image: postgres:17
    restart: unless-stopped
    environment:
      POSTGRES_USER: deskhaven
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes:
      - postgres-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD", "pg_isready", "-U", "deskhaven"]
      interval: 5s
      retries: 10
volumes:
  postgres-data:
```

Continue with step 3 of the quick start.

## An existing server

Point `database.url` at it, for example
`postgres://deskhaven:PASSWORD@db.example.com:5432/deskhaven?sslmode=verify-full`. The user
needs to create tables in that database; deskhaven creates and upgrades its schema at
startup.

deskhaven checks server certificates against the Mozilla CA list it ships with, not the
system's. For a certificate from your own CA, add `&sslrootcert=/path/to/ca.pem` to the URL.

Keep the password out of the environment with `DESKHAVEN_DATABASE__URL_FILE`
([configuration reference](../reference/configuration.md#setting-a-value)).
