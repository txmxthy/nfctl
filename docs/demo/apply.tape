# vhs tape: apply — run `just record` after `just demo-up`
Output docs/demo/apply.gif
Set Shell fish
Set FontSize 14
Set Width 1100
Set Height 420
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Type "nfctl apply -f examples/changes/fanout-add-vertex.yaml --check"
Enter
Sleep 3s
Type "nfctl apply -f examples/changes/fanout-isb-rename.yaml --check; echo exit=$status"
Enter
Sleep 3s
