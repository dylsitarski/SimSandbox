using SimSandbox.FileIO

# Read a VTK file (works with files from ParaView, other tools, etc.)
data = read_vtk("example_mesh.vtu")

# Access the data
points = data[:points]      # 3×N matrix of coordinates
cells = data[:cells]        # Vector of cell connectivities
cell_types = data[:cell_types]  # Vector of VTK cell type constants
point_data = data[:point_data]  # Dict of point-centered data
cell_data = data[:cell_data]    # Dict of cell-centered data

# Write a new VTK file
write_vtu("output_mesh.vtu", points, cells;
          cell_types=cell_types,
          point_data=point_data,
          cell_data=cell_data)

data2 = read_vtk("output_mesh.vtu")
points2 = data2[:points]
cells2 = data2[:cells]
cell_types2 = data2[:cell_types]
point_data2 = data2[:point_data]
cell_data2 = data2[:cell_data]

@assert points == points2
@assert cells == cells2
@assert cell_types == cell_types2
@assert point_data == point_data2
@assert cell_data == cell_data2