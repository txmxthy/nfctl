# Examples

Synthetic manifests for the demo cluster (`just demo-up`) and for the golden
tests. Adapted from Numaflow's own examples with neutral names; none of them
come from a real deployment.

| File | Shows |
|---|---|
| `isbsvc.yaml` | The JetStream inter-step buffer service every pipeline uses |
| `pipelines/linear.yaml` | generator → map → log; steady 5 msg/s for `top` |
| `pipelines/fanout.yaml` | conditional edges to three sinks; the `dag` showcase |
| `pipelines/scaling.yaml` | autoscaling bounds per vertex |
| `pipelines/partitions.yaml` | multi-partition edges; buffers per partition in `status` |
| `changes/fanout-add-vertex.yaml` | `apply --check` warns: topology change |
| `changes/fanout-isb-rename.yaml` | `apply --check` blocks: immutable field |
