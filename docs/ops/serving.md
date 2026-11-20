# Serving and diagnosis

In the scaffold, only the [local synthetic probes](../guides/local-development.md)
are available. HTTP 503 from readiness or inference is expected. The service
runs on IPv4 loopback and loads no assets. The production procedure below is
the LENS-1 implementation target.

1. Obtain the approved image by digest and each verified profile manifest with
   its expected digest. Install assets through the explicit importer into an
   operator-managed directory outside the repository and mount it read-only
   at a deployment-specified path such as `/models`.
2. Configure stage-to-manifest bindings and limits from the exact qualification
   receipt. Use approved device and runtime versions, service credentials and
   private connectivity. Disable serving-time egress and model downloads.
3. Check liveness, then require readiness evidence for the intended stage,
   exact served identity and successful synthetic profile probe before adding
   the worker to routing. Check Server's actual enrollment probe from the
   authenticated deployment before sending collection data.
4. On bad manifest or missing file, check the configured mount and recorded
   expected digest. Correct the import or mount and restart with the same
   approved identity. On load or probe failure, compare runtime/device and
   pinned dimensions or output shape to the receipt. Leave the worker out
   of rotation until it passes.
5. For transport or timeout failures, check service auth, routing, queue
   age, worker health and Server's remaining deadline and attempt record.
   Do not replay a submitted request from the gateway. Keep diagnostics free
   of credentials, payload text and source paths.

For replica changes or rollback use the [scaling runbook](scaling.md).
The [artifact design](../design/artifacts-and-serving.md) defines the
verification and readiness prerequisites.
