using SimSandbox
using SimSandbox.Meshing

# Generate a structured 2×2×2 mesh (8 cells, 27 nodes)
mesh = generate_structured_mesh(2, 2, 2; lx=1.0, ly=1.0, lz=1.0)

# Add a simple scalar field per cell
cell_values = [rand() for _ in mesh.cells]
point_values = [rand() for _ in 1:size(mesh.nodes, 2)]

# Write VTU
write_vtu("example_mesh.vtu", mesh; cell_data=Dict("RandomField" => cell_values), point_data=Dict("PointField" => point_values))

println("Wrote example_mesh.vtu — open it in ParaView!")
