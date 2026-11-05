# Protocol

How the apps talk to each other. This is a reference for people who want to read or extend the code; users do not need it.

Both kinds of connection use TLS with each PC's own self-signed certificate. Phones pin the PC's certificate fingerprint
(SHA-256 of the DER certificate, lower-case hex); PCs authenticate each other with mutual TLS. See "How it works" in the
[main README](../README.md) for the security model. The code is in `desktop/core` (server) and `android/app/src/main/java/dev/bridge/phone/Api.kt` (phone client).

Phone: REST over HTTPS, JSON bodies, `Authorization: Bearer <token>`. Routes: `POST /api/pair` (answer has `token`, `fingerprint` and `pc`, the PC's name), `GET /api/ping` (has `device` and `pc`), `POST /api/unpair` (the phone forgets the PC and asks the PC to forget it),
`GET|POST /api/notes` (long-poll with `since`, `del` and `wait`; the answer has `notes`, `deleted` and `del`; a note has
a shared `mid`, and `POST` accepts `reply_to`), `DELETE /api/notes/{mid}`, `POST /api/files` (answer has `state`, `"pending"`

Every phone has its own conversation with the PC. All of these calls are scoped to the authenticated device: a phone only receives the notes, deletions and files that were sent between it and the PC, and `DELETE /api/notes/{mid}`, `GET /api/files/{id}` and the download ticket answer `404` for anything that belongs to another phone. Unpairing a phone deletes its conversation.
until the PC user accepts; poll until it is `receiving`, `declined` or `cancelled`), `PUT|GET|DELETE /api/files/{id}`,
`GET /api/files/{id}/content` (HTTP Range), `POST /api/files/{id}/ticket`. The Android app uses this API.

PC to PC (mutual TLS, port 8444): `GET /peer/hello`, `POST /peer/pair`, `POST /peer/pair/{id}/reveal`, `GET /peer/pair/{id}` (long-poll),
`POST /peer/pair/{id}/complete` (sent by the starting PC after its user confirmed; only then does the answering PC save it), `POST /peer/pair/{id}/cancel`, then for paired PCs `GET /peer/ping`, `POST /peer/messages` (with `mid` and an optional `reply` object `{mid, mine, preview}` that carries the quote, so the other PC does not need the same copy of the original; repeating the same `mid` is harmless), `POST /peer/notes/delete` (`mid`),
`POST /peer/files` (sender-chosen `id`, `name`, `size`, `sha256`; answer `state` is `pending` until accepted),
`GET|PUT|DELETE /peer/files/{id}` (status, resumable upload, checksum verified; `DELETE` = cancel), `POST /peer/transfers/{id}/dismiss`
(remove a cancelled or declined card on the other PC), `POST /peer/unpair`.
