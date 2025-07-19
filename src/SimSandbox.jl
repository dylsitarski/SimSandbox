module SimSandbox

include("timesteppers/euler.jl")
include("timesteppers/rk4.jl")
#include("postprocessing/output.jl")
include("mesh-data/MeshData.jl")
include("bcs/BCs.jl")
include("utils/Utils.jl")
include("solvers/Solvers.jl")

using .Solvers
using .TimeSteppersEuler
using .TimeSteppersRK4
#using .PostProcessingOutput

const rk4_step! = TimeSteppersRK4.rk4_step!

export Convection3DSolver,
    run!,
    apply_bcs!,
    euler_step!, 
    rk4_step!
    #plot_csv_1d, 
    #plot_csv_1d_interactive,
    #plot_csv_contour

end
