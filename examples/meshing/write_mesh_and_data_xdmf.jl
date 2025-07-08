using SimSandbox.MeshData

# --- Define a 3D rectilinear mesh ---
x = range(0, 1, length=11) |> collect
y = range(0, 1, length=21) |> collect
z = range(0, 1, length=31) |> collect
mesh = Dict(
    :type => :rectilinear,
    :x => x,
    :y => y,
    :z => z
)

# --- Define time points ---
times = range(0, 1, length=11) |> collect
nt = length(times)

# --- Generate time-dependent data (moving 3D Gaussian) ---
data = Array{Float64}(undef, length(x), length(y), length(z), nt)
for (ti, t) in enumerate(times)
    xc = 0.5 + 0.2*sin(2π*t)
    yc = 0.5 + 0.2*cos(2π*t)
    zc = 0.5
    for (i, xi) in enumerate(x), (j, yj) in enumerate(y), (k, zk) in enumerate(z)
        data[i, j, k, ti] = exp(-((xi-xc)^2 + (yj-yc)^2 + (zk-zc)^2)/0.02)
    end
end

# --- Write mesh and data to separate HDF5 files ---
write_mesh("mesh3d.h5", mesh)
write_data("data3d.h5", data; name="solution")

# --- Link mesh and data with XDMF, including time list ---
write_xdmf("mesh3d.xdmf", "data3d.h5", "mesh3d.h5", mesh, data_name="solution", times=times)

println("Mesh written to mesh3d.h5, data to data3d.h5, XDMF to mesh3d.xdmf")
