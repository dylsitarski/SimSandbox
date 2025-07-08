module BCs

export BoundaryZone, BoundaryCondition, periodic_bc

struct BoundaryZone
    name::Symbol
    selector::Function  # (x, y, z) -> mask or indices
end

struct BoundaryCondition
    zone::BoundaryZone
    handler::Function   # (u, idxs) -> modifies u at idxs
end

include("periodic.jl")

end # module
