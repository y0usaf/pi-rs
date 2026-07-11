-- Exerciser for pi.exec options.onData and tool execute on_update.
local pi = ...
pi.register_tool({
  name = "streaming-demo",
  description = "Stream subprocess chunks as partial tool results",
  parameters = { type = "object", properties = {} },
  execute = function(_id, _params, _signal, on_update)
    local seen = ""
    local result = pi.exec("sh", { "-c", "printf first; sleep 0.02; printf second" }, {
      onData = function(chunk)
        seen = seen .. chunk
        if on_update then
          on_update({ content = { { type = "text", text = seen } } })
        end
      end,
    })
    return { content = { { type = "text", text = seen } }, details = { code = result.code } }
  end,
})
