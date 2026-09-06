# nfctl task runner — `just` lists recipes

demo_ctx := "orbstack"
numaflow_version := "v1.8.3"

default:
    @just --list

fmt:
    cargo fmt --all

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace --all-features

deny:
    cargo deny check

# what CI runs
ci: lint test deny

# live tests need a local demo cluster (see docs/demo/demo.md)
test-live: (_require_ctx)
    NFCTL_TEST_CONTEXT={{demo_ctx}} cargo test --workspace --all-features -- --ignored

demo-up: (_require_ctx)
    kubectl --context {{demo_ctx}} create namespace numaflow-system --dry-run=client -o yaml | kubectl --context {{demo_ctx}} apply -f -
    kubectl --context {{demo_ctx}} apply -n numaflow-system -f https://raw.githubusercontent.com/numaproj/numaflow/{{numaflow_version}}/config/install.yaml
    kubectl --context {{demo_ctx}} apply -f examples/isbsvc.yaml
    kubectl --context {{demo_ctx}} apply -f examples/pipelines/
    kubectl --context {{demo_ctx}} wait --for=jsonpath='{.status.phase}'=Running pipeline --all --timeout=300s

demo-down: (_require_ctx)
    -kubectl --context {{demo_ctx}} delete -f examples/pipelines/ --ignore-not-found
    -kubectl --context {{demo_ctx}} delete -f examples/isbsvc.yaml --ignore-not-found
    -kubectl --context {{demo_ctx}} delete namespace numaflow-system --ignore-not-found

# render docs/demo/*.tape to gifs (needs `vhs` and a release build on PATH)
record: (_require_ctx)
    cargo build --release -q
    for t in docs/demo/*.tape; do PATH="$PWD/target/release:$PATH" NFCTL_CONTEXT={{demo_ctx}} vhs "$t"; done

# refuse to touch anything but the local demo cluster
_require_ctx:
    @kubectl config get-contexts -o name | grep -qx '{{demo_ctx}}' || (echo "demo context '{{demo_ctx}}' not found" && exit 1)
