# Relay handshake fixtures

Wire frames captured from official clients (1.5.0 and 1.4.5, `--server` under Xvfb)
answering a browser-side controller through this server's rendezvous WebSocket and an
upstream hbbr 1.1.16 relay. They drive the handshake tests in `../handshake/` and
`../crypto/`.

Every identity here is **one-shot**: the server data directory and the client
configuration were created empty for the capture and deleted afterwards, the device IDs
are random, and the password was made up for the capture. `LoginResponse` and `PeerInfo`
frames were not kept, because they carry the capture host's OS user and host name; tests
that need them build synthetic ones.

Each `.b64` file is the base64 of one raw frame (base64 so Vite can load it as text).

| File                     | Frame                                                                            |
| ------------------------ | -------------------------------------------------------------------------------- |
| `hbbs-relay-response`    | `RendezvousMessage{relay_response}`; its `pk` is the server-signed `IdPk`        |
| `hbbr-signed-id`         | first relay frame from the peer: `Message{signed_id}`, signed by the device key  |
| `hbbr-public-key`        | our first relay frame: `Message{public_key}` (the capture picked `kx_version` 0) |
| `hbbr-enc-01-Hash`       | peer frame 1, sealed: `Message{hash}`                                            |
| `hbbr-enc-06-VideoFrame` | peer frame 6, sealed: one VP9 key frame (frames 2–5 not kept)                    |

`session.json` holds the capture's one-shot keys and parameters: the peer ID, the server
public key, the device signing key, both ephemeral box keys, the stream key, the
password, salt and challenge.

The 1.5.0 peer advertised `kx_version` 1 but the capturing controller picked 0, so its
sealed frames are under the version 0 key; this client picks 1 against that peer, which
the tests cover with frames sealed under the version 1 subkeys.

sha256 of the decoded frames:

| Frame                        | sha256                                                             |
| ---------------------------- | ------------------------------------------------------------------ |
| 1.5.0/hbbs-relay-response    | `dd22652ad37c1499f3828111915865c87a726cfddf1c13096cdfd57305aec994` |
| 1.5.0/hbbr-signed-id         | `b49e3ff8a45c9f6380a9436fe3ad7333a992b2421d7c69ca4c04dd0ed8c8d6e1` |
| 1.5.0/hbbr-public-key        | `ad7f6e684d975196e9b75f3280b1c73e6cc88ab0dc2154d54e6fc800595c3a11` |
| 1.5.0/hbbr-enc-01-Hash       | `9ed7c82766252e9c8cb7337bc3d0149312610a12c718519bf34bbf4771dd0fee` |
| 1.5.0/hbbr-enc-06-VideoFrame | `916613db6e597b7f50583b5ea0d99e3f6c5db27aed1ee5392a347cf259b1aac0` |
| 1.4.5/hbbs-relay-response    | `5ee7e2eab23adc6e52e0e29e019f9c462f6590b81dbcd5fc2a0a3cfa4ad21f3a` |
| 1.4.5/hbbr-signed-id         | `24641bc959467cf4a9f58b91aa8710857cc6f8570d438828b020fe5396f053a0` |
| 1.4.5/hbbr-public-key        | `e7ecd9a034f5b8cafa807377913ae9bfe774178452a1eacef709f4a2fcbca534` |
| 1.4.5/hbbr-enc-01-Hash       | `3d83dd918c7336ba80fa13426f7db13d9b411d22482a5842d50f5582a7dc0882` |
| 1.4.5/hbbr-enc-06-VideoFrame | `071bf9b6e51b00aff53d2a9bfbf9abda9fc4541f143c751f4b5e9f7082c366bd` |
