# Security

Report suspected vulnerabilities privately through GitHub's Security tab
(**Report a vulnerability**) or email **info@ioka.io** with “security” in the
subject. Include the affected revision and an authorized, sanitized
reproduction. Keep production credentials, private inputs and undisclosed
findings out of public issues and pull requests.

Lens currently has a loopback-only scaffold. The production design's
security boundary includes service authentication, exact model/artifact
verification, tenant-aware placement, and limits on queueing and execution.
Server owns collection, source and citation authorization.
[Serving design](docs/design/artifacts-and-serving.md)

If a credential enters Git or logs, treat it as compromised, rotate it and
report through the private channel. See [Support](SUPPORT.md) for public
defect reports and the current support scope.
