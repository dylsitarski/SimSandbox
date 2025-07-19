module Solvers

export Convection3DSolver, run!, apply_bcs!

# Generic boundary condition application for all solvers
function apply_bcs!(u, x, y, z, bcs)
    for bc in bcs
        bc.handler(u, x, y, z, bc.options)
    end
end

include("convection.jl")

end