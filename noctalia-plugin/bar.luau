--!nonstrict
-- valw bar widget: a camera glyph. Left click opens the toolbar panel,
-- right click takes a region shot at once (plain `valw region`, like the
-- keybind: the toolbar's timer and options don't apply).

local PANEL = "elars/valw:toolbar"

local function lastLine(text)
  local last = ""
  for line in string.gmatch(text or "", "[^\n]+") do
    last = line
  end
  return last
end

-- Exit 3 is a cancel (Esc): not an error.
local function report(result)
  if result.exitCode ~= 0 and result.exitCode ~= 3 then
    noctalia.notifyError("valw", lastLine(result.stderr))
  end
end

function update()
  barWidget.render(ui.row({ align = "center" }, {
    ui.glyph({ name = "camera" }),
  }))
  barWidget.setTooltip(noctalia.tr("bar.tooltip"))
end

function onClick()
  noctalia.togglePanel(PANEL)
end

function onRightClick()
  noctalia.runAsync("valw region", report)
end
