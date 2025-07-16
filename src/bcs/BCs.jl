module BCs

export BoundaryZone, BoundaryCondition, periodic_bc

struct BoundaryZone
    name::Symbol
    selector::Function  # (x, y, z) -> mask or indices
end

struct BoundaryCondition
    zone::BoundaryZone
    handler::Function   # (u, idxs, options) -> modifies u at idxs
    options::Dict{Symbol, Any}  # Optional parameters for the boundary condition
end

function periodic_bc(zone::BoundaryZone, connected_zone::BoundaryZone)
    handler = (u, idxs, options) -> begin
        connected_idxs = options[:connected_idxs]
        u[idxs] .= u[connected_idxs]  # Apply periodicity
    end
    options = Dict(:connected_zone => connected_zone)
    return BoundaryCondition(zone, handler, options)
end

function dirichlet_bc(zone::BoundaryZone, value::Number)
    handler = (u, idxs, options) -> begin
        u[idxs] .= options[:value]  # Set the zone to the specified value
    end
    options = Dict(:value => value)
    return BoundaryCondition(zone, handler, options)
end

end # module
