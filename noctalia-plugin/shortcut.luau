--!nonstrict
-- valw control-center tile. Click: the control center closes, then a
-- region shot (so it isn't in it). Right click: the toolbar panel.

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

shortcut.setLabel(noctalia.tr("tile.label"))
shortcut.setIcon("camera", "camera")

function onClick()
  noctalia.runAsync("noctalia msg panel-close; sleep 0.3 && valw region", report)
end

function onRightClick()
  noctalia.togglePanel(PANEL)
end
