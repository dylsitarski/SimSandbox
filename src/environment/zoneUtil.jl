# ----------------------------
# Zone utility functions
# ----------------------------
"""
    zone_indices(state, zonename)
Return indices (point or cell) described by a Zone registered in state.
"""
function zone_indices(state::State, zonename::String)
    z = state.zones[zonename]
    if z.type == :static
        return z.data  # expected Vector{Int}
    else
        error("zone_indices: dynamic zone types not yet supported")
    end
end