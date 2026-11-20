# Scale, drain and roll back

This runbook describes the production procedure to qualify under SC1/SC2.
Publish tested replica counts, device and driver revisions, profile digests,
workload limits and outcome in the candidate's
[qualification record](../design/qualification.md).

1. **Measure.** On the approved deployment with one ready worker per stage,
   run authorized interactive queries and a background rebuild. Record offered
   load, verified queries/s and builds/hour, p95/p99 latency, queue age, batch
   tokens, in-flight work, refusals, deadline timeouts, GPU memory, Server
   connections and budget reconciliation. Compare cold and warm runs and
   identify the limiting tier.
2. **Add one matching worker.** Hold Server count, build concurrency, corpus,
   candidate depth and budgets steady. If the embedding queue is the limit,
   raise that pool from one to two and keep reranking at one. Require the
   same served identity and processing approval, observe readiness, then run
   the same load. Use content-free worker counters to prove both members
   executed work. Repeat for reranking with embedding held fixed. Require
   the same citation pins and authorized outcomes under the declared bounds.
3. **Drain.** Remove a worker from routing, observe that it admits no new work,
   wait within the bounded drain interval for accepted work to finish, and
   record submitted work with an unknown or discarded outcome. Verify
   remaining capacity before termination. Exercise removal of the last ready
   dedicated worker with a shared worker still ready. Require zero forbidden
   dispatches and the configured stage outcome. Restore the approved pool.
4. **Roll back.** Restore the measured fixed replica counts and keep the
   previous immutable pool available. Use Server's revisioned controls to
   restore the earlier approved index and policy. With Lens unavailable,
   replay baseline queries in fresh sessions, verify the pins and resolve
   their citations. Reconcile each attempted call once and retain uncertainty
   where actual usage is unknown. Keep unresolved rollback outcomes visible
   in the release receipt.

[Replica design](../design/replication.md) · [Server and Lens scale qualification](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/scale-out.md#qualification-and-rollback)
