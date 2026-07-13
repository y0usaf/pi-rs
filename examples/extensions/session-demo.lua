-- Generic file-backed durable-record store exerciser.
-- The caller owns both destination directories; Rust interprets none of the
-- warehouse schema persisted here.
local pi = ...

local function required_string(value, name)
  assert(type(value) == "string" and value ~= "", name .. " is required")
  return value
end

local function drain(cursor)
  local values = {}
  local windows = 0
  repeat
    local window = cursor:next({ maxRecords = 1, maxBytes = 4096 })
    assert(#window.records <= 1, "cursor exceeded its record bound")
    windows = windows + 1
    for _, value in ipairs(window.records) do
      values[#values + 1] = value
    end
    if window.done then
      return values, windows
    end
  until false
end

pi.register_command("records-demo", {
  description = "Persist and inspect opaque records at caller-owned paths",
  handler = function(args)
    local request = pi.json.decode(args)
    local directory = required_string(request.directory, "directory")
    local copy_directory = required_string(request.copyDirectory, "copyDirectory")
    local source_name = request.sourceName or "warehouse-events"
    local copy_name = request.copyName or "warehouse-prefix"

    local source_path
    local copy_path
    local sequences = {}
    local copied_values
    local copied_windows
    local cancelled
    local count_after_cancel

    do
      local store = pi.records.create({
        directory = directory,
        name = source_name,
        maxWindowRecords = 1,
        maxWindowBytes = 4096,
      })
      local records = {
        {
          schema = "warehouse-ledger/v1",
          body = { sku = "lamp-42", adjustment = 3, labels = { "blue", "boxed" } },
        },
        {
          schema = "warehouse-ledger/v1",
          body = { sku = "cable-7", adjustment = -1, verified = true },
        },
        {
          schema = "warehouse-ledger/v1",
          body = { sku = "stand-9", adjustment = 8, bin = { aisle = 2, shelf = 4 } },
        },
      }
      for _, record in ipairs(records) do
        sequences[#sequences + 1] = store:append(record)
      end
      source_path = store:path()

      local cancellation = pi.records.cancellation()
      cancellation:cancel()
      cancelled = not pcall(function()
        store:append({ schema = "warehouse-ledger/v1", body = { unreachable = true } }, {
          cancel = cancellation,
        })
      end)
      count_after_cancel = store:record_count()

      local copied = store:copy({
        directory = copy_directory,
        name = copy_name,
        recordCount = 2,
      })
      copy_path = copied:path()
      copied_values, copied_windows = drain(copied:cursor())
    end

    -- Handles own file locks. End their scopes and finalize them before proving
    -- that the durable source can be reopened and both destinations can be listed.
    collectgarbage("collect")

    local reopened_values
    local reopened_windows
    do
      local reopened = pi.records.open({
        path = source_path,
        maxWindowRecords = 1,
        maxWindowBytes = 4096,
      })
      reopened_values, reopened_windows = drain(reopened:cursor())
    end
    collectgarbage("collect")

    local source_listing = pi.records.list({ directory = directory })
    local copy_listing = pi.records.list({ directory = copy_directory })

    return {
      sourcePath = source_path,
      copyPath = copy_path,
      sequences = sequences,
      countAfterCancel = count_after_cancel,
      cancelled = cancelled,
      reopenedValues = reopened_values,
      reopenedWindows = reopened_windows,
      copiedValues = copied_values,
      copiedWindows = copied_windows,
      sourceStores = source_listing.stores,
      sourceDiagnostics = source_listing.diagnostics,
      copyStores = copy_listing.stores,
      copyDiagnostics = copy_listing.diagnostics,
    }
  end,
})
