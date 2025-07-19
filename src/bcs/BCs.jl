module BCs

export BoundaryZone, BoundaryCondition, periodic_bc, dirichlet_bc

struct BoundaryZone
    name::Symbol
    selector::Function  # (x, y, z) -> mask or indices
end

struct BoundaryCondition
    zone::BoundaryZone
    handler::Function   # (u, x, y, z, options) -> modifies u at zone indices
    options::Dict{Symbol, Any}  # Optional parameters for the boundary condition
end

function periodic_bc(zone::BoundaryZone, connected_zone::BoundaryZone)
    handler = (u, x, y, z, options) -> begin
        idxs = zone.selector(x, y, z)
        connected_idxs = connected_zone.selector(x, y, z)
        u[idxs] .= u[connected_idxs]  # Apply periodicity
    end
    options = Dict(:connected_zone => connected_zone)
    return BoundaryCondition(zone, handler, options)
end

function dirichlet_bc(zone::BoundaryZone, value::Number)
    handler = (u, x, y, z, options) -> begin
        idxs = zone.selector(x, y, z)
        u[idxs] .= options[:value]  # Set the zone to the specified value
    end
    options = Dict(:value => value)
    return BoundaryCondition(zone, handler, options)
end

end # module
