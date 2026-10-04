--!nonstrict
-- valw bar widget: a camera glyph. Left click opens the toolbar panel,
-- right click takes a region shot at once (plain `valw region`, like the
-- keybind: the toolbar's timer and options don't apply).

local PANEL = "elars/valw:toolbar"

-- valw's stderr on an error starts with "error: <message>", then cause,
-- hint and log lines.
local function errorMessage(lines)
  for _, line in ipairs(lines) do
    local message = string.match(line, "^error: (.+)$")
    if message then
      return message
    end
  end
  return noctalia.tr("error.failed")
end

-- A capture lasts as long as the user takes (a region, a zoom, the timer),
-- so it runs as a stream: runAsync with a callback kills its command after
-- 5 s. stderr joins the stream and the exit status comes last.
local function capture(command)
  local lines = {}
  local started = noctalia.runStream("(" .. command .. ") 2>&1; echo valw-exit:$?", function(line)
    local code = string.match(line, "^valw%-exit:(%d+)$")
    if code == nil then
      table.insert(lines, line)
    elseif code ~= "0" and code ~= "3" then
      noctalia.notifyError("valw", errorMessage(lines))
    end
  end)
  if not started then
    noctalia.notifyError("valw", noctalia.tr("error.failed"))
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
  capture("valw region")
end
