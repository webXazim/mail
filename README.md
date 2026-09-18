# Harbor Mail

A React email client with a Rust backend. The frontend runs in the browser —
mail, settings, calendar, and billing persist to `localStorage`, so no backend
server is required for the client-side demo.

## Structure

```
frontend/   React + Vite web app (src/, public/, styles, config)
backend/    Rust API (Axum + SQLx + Postgres)
```

`frontend/` is self-contained: it has its own `package.json`, config files, and
`node_modules`.

## Run the frontend

```bash
cd frontend
npm install
npm run dev
```

- Dev server: http://localhost:5174 (proxies `/api` to http://localhost:8080)
- Production build: `npm run build` (outputs to `frontend/dist/`), preview with `npm run preview`

## Frontend scripts

| Command             | Purpose                              |
| ------------------- | ------------------------------------ |
| `npm run dev`       | Start the Vite dev server            |
| `npm run build`     | Type-check and build to `dist/`      |
| `npm run preview`   | Preview the production build         |
| `npm run typecheck` | Type-check the source                |
| `npm run lint`      | Lint `src/`                          |
| `npm run format`    | Format source files with Prettier    |

## Backend

Rust API in `backend/`. Run with Docker Compose from the repo root (builds from
`./backend`), or run `cargo run` inside `backend/`. See `backend/.env.example`
for configuration.