# ALICE-Chat-SaaS

Chat and messaging protocol API — part of the ALICE Eco-System.

## Overview

ALICE-Chat-SaaS delivers a high-performance chat backend: message dispatch, room management, history retrieval, and real-time presence tracking.

## Services

- **core-engine** — Messaging, rooms, history, presence (port 8127)
- **api-gateway** — JWT auth, rate limiting, reverse proxy

## Quick Start

```bash
cd services/core-engine
cargo run

curl http://localhost:8127/health
```

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | /api/v1/chat/send | Send a message |
| POST | /api/v1/chat/rooms | Create/join a room |
| GET  | /api/v1/chat/history | Retrieve message history |
| GET  | /api/v1/chat/presence | Online presence status |
| GET  | /api/v1/chat/stats | Service statistics |

## License

AGPL-3.0-or-later
