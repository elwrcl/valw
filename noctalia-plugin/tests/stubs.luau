--!nonstrict
-- Stand-ins for Noctalia's globals. Each test is run as one chunk:
-- these stubs, then the entry script, then the test (see run.sh).

calls = { run = {}, stream = {}, notify = {}, toggled = {}, closed = 0, order = {} }
fake = {
  exists = true,
  -- runAsync answers by command prefix: { exitCode, stdout, stderr }
  answers = {},
  -- runStream lines by command prefix; default: a clean exit
  streams = {},
  decoded = nil,
}
rendered = nil

local function node(kind)
  return function(props, children)
    return { kind = kind, props = props or {}, children = children or {} }
  end
end

ui = {
  row = node("row"), column = node("column"), label = node("label"),
  button = node("button"), glyph = node("glyph"), toggle = node("toggle"),
  select = node("select"), separator = node("separator"), spacer = node("spacer"),
}

noctalia = {
  tr = function(key) return key end,
  commandExists = function(_name) return fake.exists end,
  runAsync = function(command, callback)
    table.insert(calls.run, command)
    table.insert(calls.order, "run")
    if callback then
      local answer = { exitCode = 0, stdout = "", stderr = "" }
      for prefix, a in pairs(fake.answers) do
        if string.sub(command, 1, #prefix) == prefix then answer = a end
      end
      callback(answer)
    end
    return true
  end,
  -- No timeout here (runAsync with a callback kills after 5 s): captures.
  runStream = function(command, onLine)
    table.insert(calls.stream, command)
    table.insert(calls.order, "stream")
    local lines = { "valw-exit:0" }
    for prefix, l in pairs(fake.streams) do
      if string.sub(command, 1, #prefix) == prefix then lines = l end
    end
    for _, line in ipairs(lines) do onLine(line) end
    return true
  end,
  notifyError = function(title, body) table.insert(calls.notify, { title, body }) end,
  togglePanel = function(id) table.insert(calls.toggled, id) end,
  json = { decode = function(_text) return fake.decoded end },
}

panel = {
  render = function(tree) rendered = tree end,
  close = function()
    calls.closed += 1
    table.insert(calls.order, "close")
  end,
}

barWidget = {
  render = function(tree) rendered = tree end,
  setTooltip = function(_text) end,
}

shortcut = {
  setLabel = function(_text) end,
  setIcon = function(_a, _b) end,
}

-- Every node of `tree` that `match` accepts, depth first.
function findAll(tree, match, out)
  out = out or {}
  if type(tree) ~= "table" then return out end
  if tree.kind and match(tree) then table.insert(out, tree) end
  for _, child in ipairs(tree.children or {}) do findAll(child, match, out) end
  return out
end

function button(text)
  local found = findAll(rendered, function(n) return n.kind == "button" and n.props.text == text end)
  assert(#found == 1, "one button " .. text .. ", found " .. #found)
  return found[1]
end

function reset()
  calls = { run = {}, stream = {}, notify = {}, toggled = {}, closed = 0, order = {} }
end

-- valw's stderr on an error: the message first, then cause, hint and log.
function valwError(message)
  return "error: " .. message .. "\n  cause: something\n  log:   /home/u/.local/state/valw/logs/x.log\n"
end

-- The same as stream lines, ending with the exit status.
function valwFailure(message, code)
  return { "error: " .. message, "  log:   /home/u/.local/state/valw/logs/x.log", "valw-exit:" .. code }
end

function ok(name)
  print("ok - " .. name)
end
