using SimSandbox.MeshData

# --- Define a 3D cell-based unstructured mesh (hexahedra) ---
Nx, Ny, Nz = 11, 11, 11
x = range(0, 1, length=Nx) |> collect
y = range(0, 1, length=Ny) |> collect
z = range(0, 1, length=Nz) |> collect

# Node coordinates
points = [ (xi, yj, zk) for xi in x, yj in y, zk in z ]
points = reshape(points, Nx*Ny*Nz)  # flatten
points = hcat([p[1] for p in points], [p[2] for p in points], [p[3] for p in points])
num_points = size(points, 1)

# Hexahedral connectivity (VTK/Paraview node order, zero-based)
connectivity = Int[]
for i in 1:Nx-1, j in 1:Ny-1, k in 1:Nz-1
    n0 = (i-1) + (j-1)*Nz + (k-1)*Nx*Ny
    n1 = (i  ) + (j-1)*Nz + (k-1)*Nx*Ny
    n2 = (i  ) + (j  )*Nz + (k-1)*Nx*Ny
    n3 = (i-1) + (j  )*Nz + (k-1)*Nx*Ny
    n4 = (i-1) + (j-1)*Nz + (k  )*Nx*Ny
    n5 = (i  ) + (j-1)*Nz + (k  )*Nx*Ny
    n6 = (i  ) + (j  )*Nz + (k  )*Nx*Ny
    n7 = (i-1) + (j  )*Nz + (k  )*Nx*Ny
    push!(connectivity, n0, n1, n2, n3, n4, n5, n6, n7)
end
num_elements = length(connectivity) ÷ 8
connectivity = Array(reshape(connectivity, 8, num_elements)')

mesh = Dict(
    :type => :unstructured,
    :points => points,
    :connectivity => connectivity,
    :topology => "Hexahedron",
    :num_points => num_points,
    :num_elements => num_elements,
    :nodes_per_element => 8
)

# --- Define time points ---
times = range(0, 1, length=11) |> collect
nt = length(times)

# --- Generate time-dependent data (node-based, shape: num_points, nt) ---
data = Array{Float64}(undef, num_points, nt)
for (ti, t) in enumerate(times)
    xc = 0.5 + 0.2*sin(2π*t)
    yc = 0.5 + 0.2*cos(2π*t)
    zc = 0.5
    for pi in 1:num_points
        x = points[pi, 1]
        y = points[pi, 2]
        z = points[pi, 3]
        data[pi, ti] = exp(-((x-xc)^2 + (y-yc)^2 + (z-zc)^2)/0.02)
    end
end

# --- Print first 2 elements of connectivity and their node coordinates ---
println("First 2 elements of connectivity (node indices, zero-based):")
for elem in 1:min(2, num_elements)
    inds = connectivity[elem, :]
    println("  Element ", elem, ": ", inds)
    println("    Node coordinates:")
    for idx in inds
        println("      ", points[idx+1, :])  # +1 for Julia 1-based indexing
    end
end

# --- Print Julia types of arrays before writing ---
println("Array types:")
println("  points eltype: ", eltype(points))
println("  connectivity eltype: ", eltype(connectivity))
println("  data eltype: ", eltype(data))

# --- Write mesh and data to separate HDF5 files ---
write_mesh("mesh_unstructured.h5", mesh)
write_data("data_unstructured.h5", data; name="solution")

# --- Link mesh and data with XDMF, including time list ---
write_xdmf("mesh_unstructured.xdmf", "data_unstructured.h5", "mesh_unstructured.h5", mesh, data_name="solution", times=times)

# --- Also write a static (single time) XDMF for debugging ---
write_xdmf("mesh_unstructured_static.xdmf", "data_unstructured.h5", "mesh_unstructured.h5", mesh, data_name="solution")

println("Unstructured cell-based mesh written to mesh_unstructured.h5, data to data_unstructured.h5, XDMF to mesh_unstructured.xdmf and mesh_unstructured_static.xdmf")

# --- Mesh verification ---
println("Mesh verification:")
println("  points shape: ", size(points))
println("  connectivity shape: ", size(connectivity))
println("  num_elements: ", num_elements)
println("  num_points: ", num_points)

# Check connectivity indices
min_idx = minimum(connectivity)
max_idx = maximum(connectivity)
println("  connectivity min index: ", min_idx)
println("  connectivity max index: ", max_idx)
if min_idx < 0 || max_idx >= num_points
    println("  ERROR: Connectivity indices out of bounds!")
end

# Check for duplicate indices in each cell
has_duplicates = false
for elem in 1:num_elements
    inds = connectivity[elem, :]
    if length(unique(inds)) != 8
        println("  WARNING: Duplicate node indices in cell ", elem)
        has_duplicates = true
    end
end
if !has_duplicates
    println("  No duplicate node indices in any cell.")
end

println("  data shape: ", size(data))