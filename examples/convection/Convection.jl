using SimSandbox
using SimSandbox.MeshData
using SimSandbox.BCs

# 3D grid
"""
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
"""
# Read mesh from file
mesh = read_mesh("convection_mesh.h5")
x, y, z = mesh[:x], mesh[:y], mesh[:z]

# 3D initial condition: 3D sine wave
u = [sin(2π * xi) * sin(2π * yj) * sin(2π * zk) for xi in x, yj in y, zk in z]

t = range(0, 2, length=201)
t_write = range(0, 2, length=21) |> collect

# Define zones
left_face = BoundaryZone(:left, (x, y, z) -> [CartesianIndex(ix, iy, iz) for ix in findall(xi -> xi == minimum(x), x), iy in 1:length(y), iz in 1:length(z)])
right_face = BoundaryZone(:right, (x, y, z) -> [CartesianIndex(ix, iy, iz) for ix in findall(xi -> xi == maximum(x), x), iy in 1:length(y), iz in 1:length(z)])
bottom_face = BoundaryZone(:bottom, (x, y, z) -> [CartesianIndex(ix, iy, iz) for ix in 1:length(x), iy in findall(yi -> yi == minimum(y), y), iz in 1:length(z)])

# Define BCs
bcs = [
    periodic_bc(left_face, right_face),
    dirichlet_bc(bottom_face, 1.0)
]

# Create and run the 3D convection solver with RK4 time stepping
solver = Convection3DSolver(u, x, y, z, t, bcs;
    timestepper=rk4_step!,
    t_write=t_write,
    outpath="convection_out.h5",
    scheme=:upwind,
    c=(0.5, 0.0, 0.0)
)
run!(solver)

write_xdmf("convection.xdmf", "convection_out.h5", "convection_mesh.h5", mesh, data_name="solution", times=t_write)
