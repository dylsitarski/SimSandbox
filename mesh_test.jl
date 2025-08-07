using HDF5

# Define the mesh: 5 vertices and 2 tetrahedra
vertices = Float64[
    0.0 0.0 0.0;  # Vertex 1
    1.0 0.0 0.0;  # Vertex 2
    0.0 1.0 0.0;  # Vertex 3
    0.0 0.0 1.0;  # Vertex 4
    0.5 0.5 0.5   # Vertex 5
]
# Transpose vertices to match HDF5's expected layout (3x5), which will be interpreted as {5, 3} by XDMF
vertices_transposed = permutedims(vertices, (2, 1))
connectivity = Int32[0, 1, 2, 3, 1, 2, 3, 4]  # Flattened array for 2 tetrahedra (0-based indexing)
pressure = Float64[42.0, 43.0]  # Pressure for each tetrahedron

# Debug prints
println("Vertices shape: ", size(vertices))
println("Transposed vertices shape: ", size(vertices_transposed))
println("Connectivity length: ", length(connectivity))

# Write mesh to mesh.h5
h5open("mesh_2tetra.h5", "w") do file
    # Write transposed coordinates
    write(file, "/Geometry/Coordinates", vertices_transposed)  # Should result in {5, 3} when read
    # Write connectivity
    write(file, "/Topology/Connectivity", connectivity)  # 8 elements
end

# Write simulation data to data.h5
h5open("data_2tetra.h5", "w") do file
    write(file, "/Data/Pressure", pressure)
end

# Write XDMF file
xdmf_content = """
<?xml version="1.0" ?>
<Xdmf Version="3.0">
  <Domain>
    <Grid Name="TetraMesh" GridType="Uniform">
      <Topology TopologyType="Tetrahedron" NumberOfElements="2">
        <DataItem Format="HDF" DataType="Int" Dimensions="8">
          mesh_2tetra.h5:/Topology/Connectivity
        </DataItem>
      </Topology>
      <Geometry GeometryType="XYZ">
        <DataItem Format="HDF" DataType="Float" Dimensions="5 3">
          mesh_2tetra.h5:/Geometry/Coordinates
        </DataItem>
      </Geometry>
      <Attribute Name="Pressure" Center="Cell">
        <DataItem Format="HDF" DataType="Float" Dimensions="2">
          data_2tetra.h5:/Data/Pressure
        </DataItem>
      </Attribute>
    </Grid>
  </Domain>
</Xdmf>
"""

open("mesh_2tetra.xdmf", "w") do file
    write(file, xdmf_content)
end

println("Generated mesh_2tetra.h5, data_2tetra.h5, and mesh_2tetra.xdmf")