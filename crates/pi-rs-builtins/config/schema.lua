-- Configuration schema, merge policy, and per-key provenance.
--
-- One schema describes every section, and one merge walks it, so a new
-- section is a row in `SCHEMA` rather than a hand-wired special case. The
-- schema is fail-closed: an unknown key or a wrong type is an error naming
-- its dotted path, because a typo that silently does nothing is the classic
-- configuration trap.
--
-- Merge policy, by node kind:
--
-- | Kind | Higher layer wins by |
-- |---|---|
-- | `record`, `map` | merging key by key, recursively |
-- | `list` | replacing the whole sequence |
-- | `string`, `number`, `boolean` | replacing the value |
--
-- Lists replace rather than concatenate so a lower layer can never force an
-- entry a higher layer removed. Every produced leaf carries the layer and the
-- source file that produced it, which is what makes the effective
-- configuration inspectable rather than merely correct.

local pi = ...
local module = pi.kernel.v1.module

module.define({
  name = "pi.config.schema",
  version = "1",
  factory = function()
    local function scalar(kind)
      return { kind = kind }
    end

    local SCHEMA = {
      kind = "record",
      fields = {
        -- Visible product policy.
        theme = scalar("string"),
        model = {
          kind = "record",
          fields = { provider = scalar("string"), id = scalar("string") },
        },
        keymaps = { kind = "map", of = scalar("string") },

        -- Package and module selection.
        packages = { kind = "list", of = scalar("string") },
        modules = {
          kind = "list",
          of = {
            kind = "record",
            fields = { name = scalar("string"), version = scalar("string") },
          },
        },

        -- Provider and tool policy.
        providers = {
          kind = "map",
          of = {
            kind = "record",
            fields = {
              api = scalar("string"),
              base_url = scalar("string"),
              models = { kind = "list", of = scalar("string") },
            },
          },
        },
        tools = {
          kind = "record",
          fields = {
            root = scalar("string"),
            suppress = { kind = "list", of = scalar("string") },
            settings = { kind = "map", of = { kind = "map", of = scalar("string") } },
          },
        },

        -- Root selection: package identity per replaceable root.
        roots = {
          kind = "record",
          fields = {
            application = scalar("string"),
            agent = scalar("string"),
            frontend = scalar("string"),
            session = scalar("string"),
          },
        },
      },
    }

    local function is_container(node)
      return node.kind == "record" or node.kind == "map"
    end

    local function copy(value)
      if type(value) ~= "table" then
        return value
      end
      local result = {}
      for key, item in pairs(value) do
        result[key] = copy(item)
      end
      return result
    end

    local function equal(left, right)
      if type(left) ~= type(right) then
        return false
      end
      if type(left) ~= "table" then
        return left == right
      end
      for key, item in pairs(left) do
        if not equal(item, right[key]) then
          return false
        end
      end
      for key in pairs(right) do
        if left[key] == nil then
          return false
        end
      end
      return true
    end

    local function child_path(prefix, key)
      if prefix == "" then
        return tostring(key)
      end
      return prefix .. "." .. tostring(key)
    end

    local function sorted_keys(value)
      local keys = {}
      for key in pairs(value) do
        keys[#keys + 1] = key
      end
      table.sort(keys, function(left, right)
        return tostring(left) < tostring(right)
      end)
      return keys
    end

    local validate_node

    local function validate_sequence(node, value, prefix, errors)
      local count = #value
      for key in pairs(value) do
        if type(key) ~= "number" or key < 1 or key > count or key % 1 ~= 0 then
          errors[#errors + 1] = prefix .. " must be a list"
          return
        end
      end
      for index = 1, count do
        validate_node(node.of, value[index], prefix .. "[" .. index .. "]", errors)
      end
    end

    validate_node = function(node, value, prefix, errors)
      if node.kind == "record" then
        if type(value) ~= "table" then
          errors[#errors + 1] = prefix .. " must be a table"
          return
        end
        for _, key in ipairs(sorted_keys(value)) do
          local field = node.fields[key]
          if field == nil then
            errors[#errors + 1] = "unknown key " .. child_path(prefix, key)
          else
            validate_node(field, value[key], child_path(prefix, key), errors)
          end
        end
      elseif node.kind == "map" then
        if type(value) ~= "table" then
          errors[#errors + 1] = prefix .. " must be a table"
          return
        end
        for _, key in ipairs(sorted_keys(value)) do
          if type(key) ~= "string" then
            errors[#errors + 1] = prefix .. " keys must be strings"
          else
            validate_node(node.of, value[key], child_path(prefix, key), errors)
          end
        end
      elseif node.kind == "list" then
        if type(value) ~= "table" then
          errors[#errors + 1] = prefix .. " must be a list"
          return
        end
        validate_sequence(node, value, prefix, errors)
      elseif type(value) ~= node.kind then
        errors[#errors + 1] = prefix .. " must be a " .. node.kind .. ", got " .. type(value)
      elseif node.kind == "string" and value == "" then
        errors[#errors + 1] = prefix .. " must not be empty"
      end
    end

    --- Validate one layer's settings table against the schema.
    --- Returns `ok, errors` and never raises on user input.
    local function validate(settings)
      local errors = {}
      if type(settings) ~= "table" then
        return false, { "settings must be a table, got " .. type(settings) }
      end
      validate_node(SCHEMA, settings, "", errors)
      return #errors == 0, errors
    end

    local record_tree

    record_tree = function(node, value, prefix, origin, provenance)
      if is_container(node) and type(value) == "table" then
        local keys = sorted_keys(value)
        if #keys == 0 then
          provenance[prefix] = { layer = origin.layer, source = origin.source }
          return
        end
        for _, key in ipairs(keys) do
          local child = node.kind == "record" and node.fields[key] or node.of
          if child ~= nil then
            record_tree(child, value[key], child_path(prefix, key), origin, provenance)
          end
        end
        return
      end
      provenance[prefix] = { layer = origin.layer, source = origin.source }
    end

    local merge_node

    merge_node = function(node, target, overlay, prefix, origin, provenance)
      if is_container(node) and type(overlay) == "table" then
        local base = type(target) == "table" and target or {}
        local keys = sorted_keys(overlay)
        if #keys == 0 and type(target) ~= "table" then
          provenance[prefix] = { layer = origin.layer, source = origin.source }
          return {}
        end
        for _, key in ipairs(keys) do
          local child = node.kind == "record" and node.fields[key] or node.of
          if child ~= nil then
            base[key] = merge_node(
              child,
              base[key],
              overlay[key],
              child_path(prefix, key),
              origin,
              provenance
            )
            provenance[prefix] = nil
          end
        end
        return base
      end
      -- Lists and scalars replace wholesale; a replaced subtree's stale leaf
      -- provenance is dropped with it.
      for key in pairs(provenance) do
        if key == prefix or string.sub(key, 1, #prefix + 1) == prefix .. "." then
          provenance[key] = nil
        end
      end
      record_tree(node, overlay, prefix, origin, provenance)
      return copy(overlay)
    end

    --- Merge one validated layer over the accumulated settings, recording the
    --- layer and source of every produced leaf.
    local function merge(target, overlay, origin, provenance)
      return merge_node(SCHEMA, target, overlay, "", origin, provenance)
    end

    --- Every dotted leaf path present in a settings table, sorted. The
    --- inspection surface compares this against the provenance keys, so a leaf
    --- without a recorded origin is a failure rather than a silent gap.
    local function leaves(settings)
      local provenance = {}
      record_tree(SCHEMA, settings, "", { layer = "?", source = "?" }, provenance)
      return sorted_keys(provenance)
    end

    return {
      schema = SCHEMA,
      validate = validate,
      merge = merge,
      leaves = leaves,
      copy = copy,
      equal = equal,
      sorted_keys = sorted_keys,
    }
  end,
})
