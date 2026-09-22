# Operator security and RBAC

nfctl uses the Kubernetes identity from kubeconfig and has no permissions of its
own. Restrict that identity to the namespaces and commands an operator needs.
Without `-n`, discovery spans all namespaces and therefore needs cluster-scoped
authorization. For namespace isolation, grant a `Role` in each allowed namespace
and require `-n`.

All Numaflow resources below use API group `numaflow.numaproj.io`. Permissions
are additive: start with inventory access, then add only the rows for enabled
commands.

| Capability | Resource | Verbs | Used by |
|---|---|---|---|
| Pipeline inventory | `pipelines` | `get`, `list` | `ls`, `get`, `dag`, `status`, `top`, `apply`, `scale`, lifecycle commands, completion |
| Pipeline phase watch | `pipelines` | `list`, `watch` | `wait`, `pause --wait`, whole-pipeline `recycle` |
| MonoVertex inventory | `monovertices` | `get`, `list` | `ls`, `get`, `mvtx status`, completion |
| ISB inventory | `interstepbufferservices` | `list` | `isb ls`, `isb inspect`, completion |
| Pod discovery | core `pods` | `list` | logs, daemon port-forward, vertex recycle |
| Pod churn watch | core `pods` | `list`, `watch` | `logs --follow`, `mvtx logs --follow`, TUI log follow |
| Pod logs | core `pods/log` | `get` | pipeline, MonoVertex, and TUI logs |
| Daemon connection | core `pods/portforward` | `create` | runtime metrics, buffer checks, status, top, and TUI when `--daemon-url` is not set |
| Pipeline mutation | `pipelines` | `patch` | `pause`, `resume`, whole-pipeline `recycle`, server-side `apply` |
| MonoVertex mutation | `monovertices` | `patch` | `mvtx pause`, `mvtx resume` |
| Vertex scaling | `vertices/scale` | `patch` | `scale` |
| **Pod deletion** | core `pods` | `delete` | vertex `recycle`; destructive and optional |

The kube-rs watchers perform an initial list, so watches require both `list` and
`watch`. Server-side apply is sent as a `PATCH`, including when the Pipeline does
not exist; it does not require `create` or `update`. `--dry-run` changes the API
request outcome, not authorization, so it requires the same verb as a live
mutation.

## Namespace-scoped roles

This observation role covers inventory, log following, and the default daemon
port-forward. Remove `watch` when follow/wait behavior is not needed, and remove
`pods/portforward` when every daemon-backed command uses `--daemon-url`.

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: nfctl-observe
  namespace: <namespace>
rules:
  - apiGroups: [numaflow.numaproj.io]
    resources: [pipelines]
    verbs: [get, list, watch]
  - apiGroups: [numaflow.numaproj.io]
    resources: [monovertices]
    verbs: [get, list]
  - apiGroups: [numaflow.numaproj.io]
    resources: [interstepbufferservices]
    verbs: [list]
  - apiGroups: [""]
    resources: [pods]
    verbs: [list, watch]
  - apiGroups: [""]
    resources: [pods/log]
    verbs: [get]
  - apiGroups: [""]
    resources: [pods/portforward]
    verbs: [create]
```

Add mutation permissions separately so read-only users cannot change workloads:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: nfctl-mutate
  namespace: <namespace>
rules:
  - apiGroups: [numaflow.numaproj.io]
    resources: [pipelines, monovertices]
    verbs: [patch]
  - apiGroups: [numaflow.numaproj.io]
    resources: [vertices/scale]
    verbs: [patch]
```

Pod recycling is deliberately excluded. Grant it as a separate destructive
capability only to operators who may restart workloads:

```yaml
- apiGroups: [""]
  resources: [pods]
  verbs: [delete]
```

Bind these roles to the user, group, or service account that runs nfctl. For
multi-namespace access, reuse the rules in a `ClusterRole` and bind it with one
`RoleBinding` per allowed namespace. A `ClusterRoleBinding` is needed only when
nfctl must discover resources cluster-wide.

nfctl does not need access to Secrets or ConfigMaps, pod `exec`, Services, the
parent `vertices` resource, or the `create`, `update`, or `delete` verbs on
Numaflow resources.
