-- Project trust persistence/discovery mechanism exerciser.
local pi = ...

pi.register_command("trust-store-demo", {
    description = "Inspect or update the nearest project trust decision",
    handler = function(args)
        local request = pi.json.decode(args)
        if request.decision ~= nil then
            pi.trust.set(request.cwd, request.decision)
        end
        return {
            hasInputs = pi.trust.has_inputs(request.cwd),
            path = pi.trust.path(request.cwd),
            entry = pi.trust.get_entry(request.cwd),
            options = pi.trust.options(request.cwd, request.includeSessionOnly),
        }
    end,
})
