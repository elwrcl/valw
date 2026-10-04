# valw for Noctalia

A bar button, the valw toolbar as a Noctalia panel, and a control-center
tile. Noctalia draws everything, so it follows your Noctalia theme.

- Bar button: left click opens the toolbar, right click takes a region shot.
- Toolbar: click a mode to capture; the timer and options are remembered
  (shared with `valw toolbar`).
- Tile: click for a region shot, right click for the toolbar.

Install it with the home-manager module (`programs.valw.noctalia.enable`),
then enable `elars/valw` in Noctalia and place the widget (type
`elars/valw:bar`). Toggle the toolbar from a keybind with
`noctalia msg panel-toggle elars/valw:toolbar`.

Tests: `nix shell nixpkgs#luau -c sh noctalia-plugin/tests/run.sh`.
