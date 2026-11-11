# Protocol

How the apps talk to each other. This is a reference for people who want to read or extend the code; users do not need it.

Both kinds of connection use TLS with each PC's own self-signed certificate. Phones pin the PC's certificate fingerprint
(SHA-256 of the DER certificate, lower-case hex); PCs authenticate each other with mutual TLS. See "How it works" in the
[main README](../README.md) for the security model. The code is in `desktop/core` (server) and `android/app/src/main/java/dev/bridge/phone/Api.kt` (phone client).

Phone: REST over HTTPS, JSON bodies, `Authorization: Bearer <token>`. Routes: `POST /api/pair` (answer has `token`, `fingerprint` and `pc`, the PC's name), `GET /api/ping` (has `device` and `pc`), `POST /api/unpair` (the phone forgets the PC and asks the PC to forget it),
`GET|POST /api/notes` (long-poll with `since`, `del` and `wait`; the answer has `notes`, `deleted` and `del`; `since` also tells the PC what the phone has stored, and the PC's *Clear chat* deletes only notes up to there, because the phone keeps its own history; a note has
a shared `mid`, and `POST` accepts `reply_to`), `DELETE /api/notes/{mid}`, `POST /api/files` (answer has `state`, `"pending"`

Every phone has its own conversation with the PC. All of these calls are scoped to the authenticated device: a phone only receives the notes, deletions and files that were sent between it and the PC, and `DELETE /api/notes/{mid}`, `GET /api/files/{id}` and the download ticket answer `404` for anything that belongs to another phone. Unpairing a phone deletes its conversation.
until the PC user accepts; poll until it is `receiving`, `declined` or `cancelled`), `PUT|GET|DELETE /api/files/{id}`,
`GET /api/files/{id}/content` (HTTP Range), `POST /api/files/{id}/ticket`. The Android app uses this API.

**Picture previews.** A file offer (`POST /peer/files`, `POST /api/files`) may carry `thumb`: a JPEG of at most 160 pixels on a side, base64 encoded, at most 16 000 characters (the programs make about 48 pixels, around 1 to 2 KB). It is also part of a file in the history (`file.thumb`). The receiver shows it enlarged, so blurred, until the user accepts the picture. A `thumb` that is not a small, well-formed JPEG is dropped; the offer itself is still accepted.

PC to PC (mutual TLS, port 8444): `GET /peer/hello`, `POST /peer/pair`, `POST /peer/pair/{id}/reveal`, `GET /peer/pair/{id}` (long-poll),
`POST /peer/pair/{id}/complete` (sent by the starting PC after its user confirmed; only then does the answering PC save it), `POST /peer/pair/{id}/cancel`, then for paired PCs `GET /peer/ping`, `POST /peer/messages` (with `mid` and an optional `reply` object `{mid, mine, preview}` that carries the quote, so the other PC does not need the same copy of the original; repeating the same `mid` is harmless), `POST /peer/notes/delete` (`mid`),
`POST /peer/files` (sender-chosen `id`, `name`, `size`, `sha256`, and for a picture an optional `thumb`; answer `state` is `pending` until accepted),
`GET|PUT|DELETE /peer/files/{id}` (status, resumable upload, checksum verified; `DELETE` = cancel), `POST /peer/transfers/{id}/dismiss`
(remove a cancelled or declined card on the other PC), `POST /peer/unpair`.

Phone to phone, and phone to PC in the other direction, use the same `/peer/...` protocol as PC to PC, with the same mutual TLS,
pinning and six-digit comparison code, so a phone and a PC can pair either way round. The phone serves it on a port of its own
choosing (sent in the pairing request as `port`; a PC's is 8444). "Visible to other phones" only controls the mDNS announcement (finding a phone while pairing); paired devices
connect by address. A phone whose app is closed (and has *Receive while app is closed* off) does not answer; the sender keeps the message or file and retries. Phones announce themselves with mDNS as
`_bridgephone._tcp.` with the TXT records `n` (name), `fp` (certificate fingerprint, lower-case hex) and `v` (`1`); PCs use
`_bridgepc._tcp.local.`. The code is in `android/app/src/main/java/dev/bridge/phone/p2p`. A phone that does not answer a
`GET /peer/hello` with a matching fingerprint is never sent anything.
