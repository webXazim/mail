# Harbor Mail

A fully client-side React email client prototype. The entire app runs in the
browser — mail, settings, calendar, and billing persist to `localStorage`, so no
backend server is required.

## Run it

```bash
npm install
npm run dev
```

- Dev server: http://localhost:5174
- Production build: `npm run build` (outputs to `dist/`), preview with `npm run preview`

## Scripts

| Command          | Purpose                            |
| ---------------- | ---------------------------------- |
| `npm run dev`    | Start the Vite dev server          |
| `npm run build`  | Type-check and build to `dist/`    |
| `npm run preview`| Preview the production build       |
| `npm run typecheck` | Type-check the source            |
| `npm run lint`   | Lint `src/`                        |
| `npm run format` | Format source files with Prettier  |

## Structure

```
src/
  components/   Reusable UI (shell, composer, reader, layout)
  pages/        Route-level views
  services/     Client-side data services (localStorage backed)
  lib/          Domain logic and helpers
  state/        React context + reducers
  styles/       Design tokens + global stylesheet
```