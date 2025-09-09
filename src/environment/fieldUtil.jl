# ----------------------------
# Field utility functions
# ----------------------------
"""
    allocate_field(state, name, mesh_id, basis, T=Float64; init = 0.0)
Create and register a Field in `state.fields` with a default value.
"""
function allocate_field(state::State, name::String, mesh_id::String, basis::Symbol, T=Float64; init=zero(T))
    m = state.meshes[mesh_id]
    len = basis == :point ? npoints(m) : ncells(m)
    f = Field{T}(mesh_id, basis, fill(T(init), len), nothing)
    state.fields[name] = f
    return f
end

"""
    get_field(state, name)
Fetch field (throws if missing).
"""
get_field(state::State, name::String) = state.fields[name]
