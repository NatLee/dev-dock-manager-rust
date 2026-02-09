# Backend (Rust)

Backend for Dev Dock Manager. Full project docs (quick start, API, create user, dev-tool): see [README.md](../README.md) at the project root.

**Local run** (from this directory):

```bash
cargo run
```

**Create user** (local binary):

```bash
cargo run -- create-user myuser mypassword
# or with options:
cargo run -- create-user admin admin123 --email admin@example.com --staff
```

Requires Rust 1.75+. Optional: `.env` with `BIND_ADDR`, `DATABASE_URL`, `REDIS_URL`, `JWT_SECRET`, `DOCKER_NETWORK`.
