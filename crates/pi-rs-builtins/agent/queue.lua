-- Bounded FIFO queues used by the shipped agent for steering, follow-up, and
-- interrupt requests. Ordinary Lua policy: the kernel knows none of these
-- concepts, and a replacement agent may ignore or replace this module.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.agent.queue",
  version = "1",
  factory = function()
    local DEFAULT_LIMIT = 64

    local Queue = {}
    Queue.__index = Queue

    -- A queue is bounded so a stuck turn cannot grow unbounded state; a
    -- rejected push is reported to the caller instead of silently dropped.
    function Queue.new(limit)
      local bound = tonumber(limit) or DEFAULT_LIMIT
      if bound < 1 then
        bound = 1
      end
      return setmetatable({ items = {}, limit = bound }, Queue)
    end

    function Queue:len()
      return #self.items
    end

    function Queue:push(item)
      if #self.items >= self.limit then
        return false, "queue is full"
      end
      self.items[#self.items + 1] = item
      return true
    end

    function Queue:take()
      if #self.items == 0 then
        return nil
      end
      return table.remove(self.items, 1)
    end

    function Queue:drain()
      local items = self.items
      self.items = {}
      return items
    end

    function Queue:clear()
      self.items = {}
    end

    return {
      new = Queue.new,
      default_limit = DEFAULT_LIMIT,
    }
  end,
})
