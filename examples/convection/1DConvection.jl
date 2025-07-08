using SimSandbox
using SimSandbox.MeshData
using SimSandbox.BCs

# 3D grid
x = range(0, 1, length=61) |> collect
y = range(0, 1, length=51) |> collect
z = range(0, 1, length=41) |> collect
mesh = Dict(
    :type => :rectilinear,
    :x => x,
    :y => y,
    :z => z
)

write_mesh("convection_mesh.h5", mesh)

# 3D initial condition: 3D sine wave
u = [sin(2π * xi) * sin(2π * yj) * sin(2π * zk) for xi in x, yj in y, zk in z]

t = range(0, 2, length=201)
t_write = range(0, 2, length=21) |> collect

# Define zones
left_face = BoundaryZone(:left, (x, y, z) -> findall(xi -> xi == minimum(x), x))
right_face = BoundaryZone(:right, (x, y, z) -> findall(xi -> xi == maximum(x), x))

# Define BCs
bcs = [
    BoundaryCondition(left_face, (u, idxs) -> u[idxs, :, :] .= 0.0),
    BoundaryCondition(right_face, periodic_bc)
]

# Create and run the 3D convection solver with RK4 time stepping
solver = Convection3DSolver(u, x, y, z, t, bcs;
    timestepper=rk4_step!,
    t_write=t_write,
    outpath="convection3d_output.h5",
    scheme=:upwind,
    c=(0.5, 0.5, 0.0)
)
run!(solver)

write_xdmf("convection.xdmf", "convection3d_output.h5", "convection_mesh.h5", mesh, data_name="solution", times=t_write)
