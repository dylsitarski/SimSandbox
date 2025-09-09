using SimSandbox.Environment
using SimSandbox.FileIO

# Read a VTK file (returns a State containing a mesh keyed by "default")
st = read_vtk("example_mesh.vtu")
mesh = st.meshes["default"]

# Access the data
points = mesh.points
cells = mesh.cells
cell_types = mesh.cell_types
point_data = Dict{String,Any}()
cell_data = Dict{String,Any}()
for (k,f) in st.fields
    if f.basis == :point
        point_data[k] = f.data
    else
        cell_data[k] = f.data
    end
end

# Write a new VTK file using the State API
write_vtu(st, "output_mesh.vtu")

# Read back
st2 = read_vtk("output_mesh.vtu")
mesh2 = st2.meshes["default"]
points2 = mesh2.points
cells2 = mesh2.cells
cell_types2 = mesh2.cell_types
point_data2 = Dict{String,Any}()
cell_data2 = Dict{String,Any}()
for (k,f) in st2.fields
    if f.basis == :point
        point_data2[k] = f.data
    else
        cell_data2[k] = f.data
    end
end

@assert points == points2
@assert cells == cells2
@assert cell_types == cell_types2
@assert point_data == point_data2
@assert cell_data == cell_data2