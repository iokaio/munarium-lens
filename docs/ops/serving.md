# Serving and diagnosis

LENS-1 currently serves only the deterministic reference backend, which is
not a retrieval model and loads no assets. Use the [local walkthrough](../guides/local-development.md)
for its probes. Do not enroll a collection against it: readiness and
discovery report `"retrieval_model": false` for that reason. Service
authentication, artifact import and a model runtime are pending, so the
procedure below remains the target for the model-backed image.

1. Obtain the approved image by digest and each verified profile manifest with
   its expected digest. Install assets through the explicit importer into an
   operator-managed directory outside the repository and mount it read-only
   at a deployment-specified path such as `/models`.
2. Configure stage-to-manifest bindings and limits from the exact qualification
   receipt. Use approved device and runtime versions, service credentials and
   private connectivity. Disable serving-time egress and model downloads.
3. Check liveness, then require readiness evidence for the intended stage,
   exact served identity and successful synthetic profile probe before adding
   the worker to routing. `GET /readyz` returns HTTP 200 only when every
   configured stage passed its startup probe, and lists each ready stage's
   model, backend, dimensions, manifest digest and limits. Check Server's actual enrollment probe from the
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
