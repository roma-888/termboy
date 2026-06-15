# termboy demo recording.
#
# Produces assets/demo.gif (referenced from the README).
#
# Requires charmbracelet/vhs:  brew install vhs
# Pre-build first so the recording starts instantly (no compile on screen):
#     cargo build --release -p termboy
# Then:
#     vhs assets/demo.tape
#
# Swap roms/Tetris.gb below for whatever you'd rather show off.

Output assets/demo.gif

Set FontSize 16
Set Width 1000
Set Height 640
Set Padding 16
Set Theme "Catppuccin Mocha"

# Launch off-screen so the GIF opens straight into the game.
Hide
Type "./target/release/termboy roms/Tetris.gb"
Enter
Sleep 3s
Show

# Title screen, then start and play a little.
Sleep 2s
Enter
Sleep 2s
Left 3
Sleep 500ms
Right 4
Sleep 500ms
Down@250ms 6
Sleep 3s
Escape
