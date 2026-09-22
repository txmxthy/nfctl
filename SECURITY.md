# Security policy

## Supported versions

nfctl is pre-alpha and has no tagged release. Security fixes target the current
default branch. Once releases begin, only the latest published release and the
default branch will receive security fixes until 1.0.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: open the repository's
[Security advisories](https://github.com/txmxthy/nfctl/security/advisories/new)
page and select **Report a vulnerability**. Do not include vulnerability details
in a public issue. If GitHub reports that private reporting is unavailable, do
not disclose the details publicly; wait for the maintainers to publish a private
contact route.

Include the affected version, impact, reproduction steps, and any known
workaround. Please allow the maintainers time to investigate before disclosure.

## Security boundary

nfctl runs with the authority of the Kubernetes identity selected from the
current kubeconfig. It does not install an in-cluster service account or elevate
that identity. Operators should grant only the commands they intend to expose;
the exact API permissions are documented in
[Operator security and RBAC](docs/operator-security.md).

The default daemon connection is an API-server-authorized port-forward to a
selected Numaflow daemon pod. The daemon creates a new self-signed certificate
at startup, so nfctl encrypts this inner connection but does not authenticate
that certificate. A direct `--daemon-url http://...` connection is plaintext;
the current `https://...` direct connection also does not verify the server
certificate. Use direct URLs only on a trusted path.

Command output can contain manifests, resource names, health data, and pod logs.
Treat redirected output and fixture files according to the sensitivity of the
cluster data they contain.
