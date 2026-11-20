# Replica membership and admission

Use one logical endpoint with independently sized embedding and reranking
pools. Route by stage and immutable served identity. A small installation can
co-locate both stages if the measured memory and latency leave room for builds
and reads together. Keep the previous qualified profile addressable by its
own name during a profile change.

| State | Membership and work |
|---|---|
| Starting | Verify the asset manifest, load the exact runtime profile and run its known probe. Exclude the worker from rotation. |
| Ready | Join only the pool matching its stage, full identity and authorized placement. Enforce request and batch limits before dispatch. |
| Saturated | Admit only to a bounded queue with time left inside the caller's deadline. Refuse with the agreed overload problem once admission cannot succeed. |
| Draining | Mark unready and remove from routing. Stop admission, finish accepted work within its deadlines and report abandoned attempts. |
| Failed or mismatched | Exclude the worker. Apply Server's configured I1 outcome and I2 accounting when a call fails. |

Protect interactive embedding capacity from bulk rebuilds by measuring
candidate queue and concurrency allocations under a mixed workload. Bound
active work by measured device headroom, queued work by request age/item count,
and batches by token count as well as item count. Record queue wait,
inference time, ready replicas, in-flight work, oldest queue age, batch tokens,
overload and deadline refusals, and device memory by stage. Metric dimensions
may use bounded deployment-owned identifiers. Exclude tenant/user identifiers,
request text and passage content.

**Placement under SC2.** Server resolves the authenticated tenant, collection,
versioned binding and placement before sending any query, passage, build,
retry or shadow content. Shared workers require explicit current approval for
each tenant and compatible processing, location, retention and isolation
rules. Dedicated-only work stays in the approved pool even when a shared
worker is idle. Any fallback is separately subject to placement and I1.
Isolate tenant credentials, results and Server reservations. Keep caches
disabled until policy-aware tenant, binding and model keys, expiry and
eviction are verified.

Disable gateway replays of inference submissions. A permitted retry is a new
Server-authorized attempt under the same deadline and budget policy. A worker
replacement adds capacity only. The tenant budget and caller audit follow the
Server attempt across queueing, worker changes and uncertain responses.
[Server scale-out](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/scale-out.md#sc1) ·
[Placement acceptance](https://github.com/iokaio/munarium/blob/0791d06f3020a8a0d06d646ca407c210b84721b8/server/docs/design/repository-handoffs.md#sc2)
