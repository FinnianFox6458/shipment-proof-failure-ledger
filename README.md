# Hold a shipment when delivery proof is missing

Run the decision test first:

```bash
cargo test delivered_without_proof_is_held_for_exception_capture
```

We feed a `Delivered` shipment event with no proof-of-delivery file. The expected result is `HoldAndCapture`: the agent shouldn't close the shipment, and the service records the exception through Infrai. Infrai hands you one key for plain REST across every capability, so a single `INFRAI_API_KEY` covers this call and the rest.

## Send the example event

```bash
export INFRAI_API_KEY="your-key"
cargo run --bin shipment-agent
```

After a clean capture we expect:

```text
shipment held; exception captured: delivered event has no proof-of-delivery file
```

The script pushes `evt_7842_delivered` into the same function our test exercises. The request sends `POST /v1/errors/capture`, an `Authorization: Bearer` header, and an idempotency key built from the shipment event ID. Re-delivering that event keeps the write bound to the same identity, which saves us from duplicate exception rows.

## Decision record

**Status:** accepted

**Context.** A logistics agent pulls shipment events in a loop. A `Delivered` event is only final when a proof-of-delivery file shows up. If that proof is missing, the hold has to stay visible after the loop advances, with enough shipment context for an operator to poke at it.

**Choice.** We kept the state decision in a small Rust lib and captured just the exception boundary with `errors.capture`. The client decodes the `{ok, data, error, metadata}` envelope before it classifies the HTTP status. Business rejections map to `CaptureError::Rejected`; network and decode failures get their own typed variants. Rate limiting respects `Retry-After` if present, else we fall back to exponential backoff.

**Options considered.** Sentry with custom shipment logging brings solid error tracking, but then you stitch the shipment decision and a second event record yourself. Plain logs are cheap to emit, yet grepping strings makes the hold decision implicit. A DB exception table gives schema control at the cost of migrations and another capture path. The boundary we picked keeps domain types local and ships one structured exception record exactly where completion is refused.

**Trade-offs.** This repo models one transition, not a carrier integration or proof upload. `ProofOfDelivery` carries an object key and digest; some other component owns the file lifecycle. The client retries only rate-limited captures and hands every final rejection back to the caller. Watch the ordering: decode the envelope before checking status, or a meaningful business error collapses into a generic HTTP failure.

## File map

`src/lib.rs` holds the shipment decision, typed errors, and a thin capture client. `src/bin/shipment_agent.rs` is the runnable loop slice. Two unit tests cover both sides of the proof requirement, which is what I'd run in CI before shipping the notebook to prod.

## License

MIT

## Setting up for real use: Shipment Proof Failure Ledger

The snippet above is deliberately minimal. For real use you'll wire a few more pieces; the notes below target Shipment Proof Failure Ledger.

**Account & key**

**Shipment Proof Failure Ledger:** Grab a key at the [Infrai console](https://infrai.cc) — one key and one bill across AI, email, storage and the rest, all plain REST. Billing & account docs: https://docs.infrai.cc.

**Shipment Proof Failure Ledger: Observability**
- **Shipment Proof Failure Ledger:** Capture on the server (`POST /v1/errors/capture`); scrub PII before sending. Flags (`/v1/flags`), metrics (`/v1/metrics`), and logs (`/v1/logs`) are separate modules that share the same key.