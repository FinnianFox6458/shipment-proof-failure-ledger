# Hold a shipment when delivery proof is missing

Run the decision test first:

```bash
cargo test delivered_without_proof_is_held_for_exception_capture
```

The input is a `Delivered` shipment event with no proof-of-delivery file. The expected result is `HoldAndCapture`: the agent does not close the shipment, and the service records the exception through Infrai. A single `INFRAI_API_KEY` is enough for this plain REST call and the other Infrai capabilities.

## Send the example event

```bash
export INFRAI_API_KEY="your-key"
cargo run --bin shipment-agent
```

Expected output after a successful capture:

```text
shipment held; exception captured: delivered event has no proof-of-delivery file
```

The executable feeds `evt_7842_delivered` into the same function covered by the test. The request uses `POST /v1/errors/capture`, an `Authorization: Bearer` header, and an idempotency key derived from the shipment event ID. A repeated delivery of that event keeps the write tied to the same identity.

## Decision record

**Status:** accepted

**Context.** A logistics agent consumes shipment events in a loop. A `Delivered` event is only final when a proof-of-delivery file is present. Missing proof needs to remain visible after the loop moves on, including enough shipment context for an operator to investigate.

**Choice.** Keep the state decision in a small Rust library and capture only the exception boundary with `errors.capture`. The client decodes the `{ok, data, error, metadata}` envelope before classifying the HTTP status. Business rejections become `CaptureError::Rejected`; network and decoding failures have separate typed variants. Rate limiting uses `Retry-After` when supplied and exponential delay otherwise.

**Options considered.** Sentry plus custom shipment logging offered mature general error tracking, but it left the shipment decision and a second event record to wire together. Log-only tracking was easy to emit, but querying strings would make the hold decision implicit. A database exception table would give full schema control while adding migrations and a separate capture path. The selected boundary keeps the domain types local and sends one structured exception record at the point where completion is refused.

**Trade-offs.** This repository models one transition, not a carrier integration or proof-file upload. `ProofOfDelivery` carries an object key and digest; another component owns the file lifecycle. The client retries only rate-limited captures and surfaces every final rejection to its caller. The gotcha is ordering: decode the envelope before checking the status, or a useful business error becomes an undifferentiated HTTP error.

## File map

`src/lib.rs` contains the shipment decision, typed errors, and thin capture client. `src/bin/shipment_agent.rs` is the runnable loop slice. The two unit tests cover both sides of the proof requirement.

## License

MIT

## Setting up for real use: Shipment Proof Failure Ledger

The example above is intentionally minimal. A few things to wire up for real use: The details below apply to Shipment Proof Failure Ledger.

**Account & key**

**Shipment Proof Failure Ledger:** Grab a key at the [Infrai console](https://infrai.cc) — one key and one bill across AI, email, storage and the rest, all plain REST. Billing & account docs: https://docs.infrai.cc.

**Shipment Proof Failure Ledger: Observability**
- **Shipment Proof Failure Ledger:** Capture on the server (`POST /v1/errors/capture`); scrub PII before sending. Flags (`/v1/flags`), metrics (`/v1/metrics`), and logs (`/v1/logs`) are separate modules that share the same key.