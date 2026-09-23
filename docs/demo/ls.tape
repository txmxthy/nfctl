# vhs tape: ls — deterministic, runs against examples/fixtures/demo.yaml (no cluster)
Output docs/demo/ls.gif
Set Shell fish
Set FontSize 14
Set Width 1100
Set Height 320
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Env NFCTL_FIXTURE examples/fixtures/demo.yaml
Type "nfctl ls"
Enter
Sleep 2s
Type "nfctl ls -o json | jq '.[].status.phase'"
Enter
Sleep 3s
