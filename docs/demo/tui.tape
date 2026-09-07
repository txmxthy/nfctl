# vhs tape: tui — deterministic, runs against examples/fixtures/demo.yaml (no cluster)
Output docs/demo/tui.gif
Set Shell fish
Set FontSize 14
Set Width 1100
Set Height 600
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Env NFCTL_FIXTURE examples/fixtures/demo.yaml
Type "nfctl tui"
Enter
Sleep 2s
Screenshot docs/demo/tui-pipelines.png
Sleep 1s
Enter
Sleep 2s
Screenshot docs/demo/tui-detail.png
Sleep 1s
Type "j"
Sleep 500ms
Enter
Sleep 2s
Screenshot docs/demo/tui-logs.png
Sleep 1s
Escape
Sleep 500ms
Escape
Sleep 500ms
Type "q"
Sleep 500ms
