--!nonstrict
-- valw toolbar panel: the four modes, the timer and the options, drawn
-- with Noctalia's own components. The choices live in valw's toolbar.toml:
-- read with `valw toolbar --state`, changed with `valw toolbar --set`.
-- A mode click closes the panel, waits for it to go, then captures.

local MODES = {
  { id = "screen", glyph = "device-desktop" },
  { id = "region", glyph = "crop" },
  { id = "window", glyph = "app-window" },
  { id = "zoom", glyph = "zoom-in" },
}
local OPTIONS = { "cursor", "preview", "sound" }
local KEYS = { "mode", "timer", "cursor", "preview", "sound" }

-- The decoded `--state` JSON; nil until it arrives.
local state = nil
local unavailable = false

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

local function linesOf(text)
  local lines = {}
  for line in string.gmatch(text or "", "[^\n]+") do
    table.insert(lines, line)
  end
  return lines
end

-- Exit 3 is a cancel (Esc): not an error.
local function report(result)
  if result.exitCode ~= 0 and result.exitCode ~= 3 then
    noctalia.notifyError("valw", errorMessage(linesOf(result.stderr)))
  end
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

local render

-- Every choice goes with every --set, not just the changed one: each
-- --set loads, changes and saves the file, so two quick changes sending
-- one key each could undo each other. The values are valw's own tokens
-- (from --state or our lists), never user text.
local function remember(key, value)
  state[key] = value
  render()
  local sets = {}
  for _, k in ipairs(KEYS) do
    table.insert(sets, "--set " .. k .. "=" .. tostring(state[k]))
  end
  noctalia.runAsync("valw toolbar " .. table.concat(sets, " "), report)
end

local function modeButton(m)
  return ui.button({
    text = noctalia.tr("mode." .. m.id),
    glyph = m.glyph,
    variant = if state.mode == m.id then "primary" else "outline",
    onClick = function()
      panel.close()
      capture("sleep 0.3 && valw toolbar --run " .. m.id)
    end,
  })
end

local function optionRow(key)
  return ui.row({ gap = 8, align = "center", justify = "space_between" }, {
    ui.label({ text = noctalia.tr("option." .. key), color = "on_surface" }),
    ui.toggle({
      checked = state[key],
      onChange = function(value)
        remember(key, value == "true")
      end,
    }),
  })
end

render = function()
  if unavailable then
    panel.render(ui.column({ padding = 16 }, {
      ui.label({ text = noctalia.tr("panel.unavailable"), color = "error" }),
    }))
    return
  end
  if state == nil then
    return
  end
  local modes = {}
  for _, m in ipairs(MODES) do
    table.insert(modes, modeButton(m))
  end
  local timers, selected = {}, 0
  for i, t in ipairs(state.timers) do
    table.insert(timers, if t == 0 then noctalia.tr("timer.off") else tostring(t) .. " s")
    if t == state.timer then
      selected = i - 1
    end
  end
  local rows = {
    ui.row({ gap = 8, justify = "space_between" }, modes),
    ui.separator({}),
    ui.row({ gap = 8, align = "center", justify = "space_between" }, {
      ui.label({ text = noctalia.tr("timer.label"), color = "on_surface" }),
      ui.select({
        options = timers,
        selectedIndex = selected,
        onChange = function(index)
          remember("timer", state.timers[tonumber(index) + 1])
        end,
      }),
    }),
  }
  for _, key in ipairs(OPTIONS) do
    table.insert(rows, optionRow(key))
  end
  panel.render(ui.column({ gap = 12, padding = 16 }, rows))
end

local function fail(message)
  unavailable = true
  noctalia.notifyError("valw", message)
  render()
end

function onOpen(_context)
  unavailable = false
  if not noctalia.commandExists("valw") then
    fail(noctalia.tr("panel.unavailable"))
    return
  end
  noctalia.runAsync("valw toolbar --state", function(result)
    if result.exitCode ~= 0 then
      fail(errorMessage(linesOf(result.stderr)))
      return
    end
    local decoded = noctalia.json.decode(result.stdout)
    if type(decoded) ~= "table" then
      fail(noctalia.tr("panel.unavailable"))
      return
    end
    state = decoded
    render()
  end)
end
