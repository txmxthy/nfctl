# vhs tape: dag — deterministic, runs against examples/fixtures/demo.yaml (no cluster)
Output docs/demo/dag.gif
Set Shell fish
Set FontSize 14
Set Width 1100
Set Height 520
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Env NFCTL_FIXTURE examples/fixtures/demo.yaml
Type "nfctl dag fanout"
Enter
Sleep 2s
Screenshot docs/demo/dag.png
Sleep 1s
Type "nfctl dag fanout -f mermaid"
Enter
Sleep 3s
