# vhs tape: pause — run `just record` after `just demo-up`
Output docs/demo/pause.gif
Set Shell fish
Set FontSize 14
Set Width 1100
Set Height 380
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Type "nfctl pause linear --wait"
Enter
Sleep 15s
Type "nfctl status linear | head -3"
Enter
Sleep 2s
Type "nfctl resume linear --strategy slow"
Enter
Sleep 2s
