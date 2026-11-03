<div align="center">

# **Dev Dock Manager (Rust)**

<p>
  <img src="./doc/workflow.gif" alt="Logging in, creating a container, opening its web terminal and its desktop in NoVNC">
</p>

<a href="https://github.com/NatLee/dev-dock-manager-rust/actions/workflows/backend.yml"><img src="https://github.com/NatLee/dev-dock-manager-rust/actions/workflows/backend.yml/badge.svg" alt="Backend CI"></a>
<a href="./LICENSE"><img src="https://img.shields.io/github/license/NatLee/dev-dock-manager-rust" alt="License: MIT"></a>

</div>

Dev Dock Manager is a small self-hosted dashboard for handing out Linux desktops on one machine. Each person
logs in, creates a container from the [dev-dock](https://github.com/NatLee/dev-dock) image, and gets a full
Xfce desktop in the browser (NoVNC), a web terminal and an SSH port, without touching Docker themselves. This
version has a Rust API (axum + bollard) and a Next.js frontend, all behind Traefik.

## The Dev Dock family

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/family-dark.webp">
    <img src="docs/images/family-light.webp" alt="The Dev Dock family: dev-dock builds the gui-vnc desktop image; dev-dock-manager (Django) and dev-dock-manager-rust (Rust) include it as the gui/ submodule and start one gui-vnc container per environment through the Docker socket">
  </picture>
</p>

| Repo | What it is |
|---|---|
| [dev-dock](https://github.com/NatLee/dev-dock) | The `gui-vnc` image: Xfce desktop, VNC, noVNC and SSH in one container. Included here as `gui/`. |
| [dev-dock-manager](https://github.com/NatLee/dev-dock-manager) | The original manager, with a Django (Python) API. |
| **dev-dock-manager-rust** (this repo) | The same dashboard, routes and container setup, with the API rewritten in Rust. |

The two managers look the same from the browser and start the same containers, so pick one per machine.
What differs is the backend:

| | dev-dock-manager-rust | dev-dock-manager |
|---|---|---|
| API | one Rust binary (axum, bollard) | Django + Daphne, run by supervisord |
| Background jobs | a worker task inside the API process, Redis list as the queue | a separate `rqworker` process (RQ) |
| Users | `users` table in SQLite, Argon2 password hashes | Django users in SQLite |
| Login tokens | access 1 hour, refresh 1 day | access 60 hours, refresh 1 day |
| Extras | `/health` | Django admin and Swagger pages |

## How it works

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/architecture-dark.webp">
    <img src="docs/images/architecture-light.webp" alt="Browser to Traefik on port 8000. Traefik sends /api, /dashboard/api and /ws to the Rust backend (priority 10), /novnc/name/ to that desktop container's noVNC on 6901 with the prefix stripped, and everything else to the Next.js frontend on 3000. The backend uses Redis, SQLite and the Docker socket; SSH goes straight to each container's host port">
  </picture>
</p>

Everything you open goes through **Traefik** on port 8000, which picks the target by path:

| Path | Goes to |
|---|---|
| `/api/*`, `/dashboard/api/*`, `/ws/*` | the Rust API (`d-gui-manager-backend`, port 8000) |
| `/novnc/<name>/` | the noVNC page of the desktop called `<name>` (port 6901), with the prefix stripped |
| everything else | the Next.js dashboard (`d-gui-manager-frontend`, port 3000) |

Traefik learns these routes from Docker labels. The services get theirs from `docker-compose.yml`, and each
desktop gets its own when the API creates it. All of them share the external network `d-gui-network`. SSH is
the one thing that skips Traefik: each desktop publishes port 22 on a host port you choose.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/lifecycle-dark.webp">
    <img src="docs/images/lifecycle-light.webp" alt="Sequence: 1 log in with POST /api/auth/token; 2 create with POST /dashboard/api/container/new, the API queues a job and returns a task_id; 3 the worker creates and starts the gui-vnc container; 4 CREATED arrives on /ws/notifications/; 5 the console WebSocket /ws/console/ runs docker exec bash or attach; 6 NoVNC through Traefik; 7 start, stop, restart, remove through /dashboard/api/containers/control">
  </picture>
</p>

What each click does:

1. **Log in.** The dashboard posts your username and password to `/api/auth/token` and keeps the access and
   refresh tokens (JWT) in the browser's localStorage. Every API call sends the access token.
2. **Create.** The API checks the name (two characters or more, starting with a letter) and that the SSH port
   is free, puts a job on the Redis queue and answers with a `task_id` right away.
3. **The worker starts the container** from `gui-vnc`: your passwords as environment variables, port 22 on
   the SSH port you picked, `/etc/localtime` mounted read-only, the Traefik labels for `/novnc/<name>/`, and
   optionally privileged mode or all NVIDIA GPUs.
4. **The list refreshes** when `CREATED` arrives on the `/ws/notifications/` WebSocket.
5. **Console** opens a WebSocket to `/ws/console/?container=<id>`, sends the token in the first message and
   gets a `bash` started with `docker exec` (a root shell). **Attach** connects to the container's main
   process instead. Keys and window size go up as `pty_input` and `pty_resize`.
6. **NoVNC** opens `/novnc/<name>/` through Traefik and asks for the VNC password you set when creating it.
7. **Start, stop, restart, remove** go through the same queue. You see `WAITING`, then `STARTED`, `STOPPED` and so on.

## Interface

- **Login**
  ![login](./doc/login.gif)

- **Create container**
  ![create](./doc/create.gif)

- **Web terminal**
  ![web terminal](./doc/console.gif)

- **NoVNC**
  ![novnc](./doc/novnc.gif)

## Quick start

You need Linux with Docker running (the API talks to `/var/run/docker.sock`). On Windows, use Docker inside WSL.

1. **Get the code with the image submodule:**

   ```bash
   git clone --recurse-submodules https://github.com/NatLee/dev-dock-manager-rust.git
   cd dev-dock-manager-rust
   # already cloned? git submodule update --init --recursive
   ```

2. **Build the desktop image** (it ends up as `gui-vnc`; a first build takes 10 to 30 minutes and about 7.8 GB):

   ```bash
   cd gui && docker compose build && cd ..
   ```

   Or pull a pre-built one from [natlee/gui-vnc](https://hub.docker.com/r/natlee/gui-vnc) and name it
   `gui-vnc`, because that's the name the API starts containers from:

   ```bash
   docker pull natlee/gui-vnc:20260208-arm64
   docker tag natlee/gui-vnc:20260208-arm64 gui-vnc
   ```

3. **Create the network and start the stack:**

   ```bash
   docker network create d-gui-network
   docker compose build && docker compose up -d
   ```

   The `nvidia-cuda` service only prints "CUDA image ready" and exits.

4. **Create the first user:**

   ```bash
   ./dev-tool.sh create-superuser
   ```

   It asks for a username and password (defaults `admin` / `1234`). To skip the prompts or pass options:

   ```bash
   docker exec -it d-gui-manager-backend /app/dev-dock-manager-api create-user admin mypassword --staff
   ```

5. **Open http://localhost:8000/** and log in. It sends you to `/dashboard/containers`.

## Good to know

- **Opening it from another machine.** The dashboard calls the API at `NEXT_PUBLIC_API_ORIGIN` from
  `frontend/.env.local`, which is `http://localhost:8000`, and Next.js bakes that in at build time. For any
  other address, set it to that address, or leave it empty (`NEXT_PUBLIC_API_ORIGIN=`) so the dashboard uses
  whatever address it was opened from. Then run `docker compose build frontend && docker compose up -d`.
- **Ports.** 8000 is the app, 8080 is the Traefik dashboard. Each desktop also takes the SSH port you pick.
- **Data.** Users live in `./backend-data/db.sqlite3`. The desktops are ordinary containers named after what
  you typed, so `docker ps` shows them.
- **Keep the image name `gui-vnc`.** The API lists, creates and manages containers whose image starts with
  `gui-vnc`. `DOCKER_IMAGE_NAME` only changes which images the dashboard lists.
- **Stopping.** The API handles SIGTERM and SIGINT, so `docker compose stop` ends it cleanly in about a second.

### Before you put it on a network

It's built for a trusted group on one machine. Every account can see and control every desktop (there's no
per-user ownership), and can create privileged containers. With the Docker socket mounted, that amounts to
root on the host. Also:

- change `JWT_SECRET` in `docker-compose.yml` (the default is `change-me-in-production`);
- the Traefik dashboard on 8080 runs with `--api.insecure`, so don't expose that port.

## NVIDIA GPU

Optional. With NVIDIA drivers and the NVIDIA Container Toolkit on the host, the create dialog lets you give a
container all GPUs (`/dashboard/api/nvdocker/check` tells the dashboard whether that works).

## Dev helper script

From the project root:

```bash
./dev-tool.sh --help
```

- `bash`: shell in the backend container (`d-gui-manager-backend`).
- `create-superuser`: create a login user (Rust `create-user`).
- `logs`: follow backend logs.
- `backend-debug`: run the backend in a one-off container with `RUST_LOG=debug`.

## API (backend)

- **Prefix**: `/api`, and the same routes again under `/dashboard/api`
  (e.g. `/api/auth/token`, `/dashboard/api/containers`, `/dashboard/api/ports`).
- **Health**: `GET /health` → `ok`, on the API itself (port 8000 inside the Docker network). Traefik doesn't
  route `/health`, so `localhost:8000/health` from the host reaches the frontend and returns 404.
- **Web terminal**: the frontend calls `GET /dashboard/api/console/:action/:id` for metadata, then opens the
  WebSocket `/ws/console/?container=<container_id>`. The first message must carry the JWT:
  `{"token": "<access token>", "action": "shell" | "attach", "payload": {"Id": "<container_id>"}}`.
  After that, `pty_input` and `pty_resize` messages carry keys and window size.
- **Notifications**: `/ws/notifications/` pushes `{"message": {"action": "CREATED" | "STARTED" | …, "details": "…"}}`.

**Backend layout**: `backend/src/api/` (auth, containers, images, ports), `backend/src/ws/` (console and
notifications), `backend/src/docker/` (bollard), `backend/src/queue/` (Redis queue and worker),
`backend/src/db/` (users, SQLite + Argon2). See [backend/README.md](backend/README.md) for running it locally.

## License

[MIT](./LICENSE)
