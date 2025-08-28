using SimSandbox.MeshData

# --- Define a 3D curvilinear mesh ---
nx, ny, nz = 11, 21, 31
x1d = range(0, 1, length=nx) |> collect
y1d = range(0, 1, length=ny) |> collect
z1d = range(0, 1, length=nz) |> collect
coordinates = zeros(Float64, nx, ny, nz, 3)
for ix in 1:nx, iy in 1:ny, iz in 1:nz
    coordinates[ix, iy, iz, 1] = x1d[ix] + 0.1 * sin(π * y1d[iy]) * sin(π * z1d[iz])  # Perturbation for curvilinear
    coordinates[ix, iy, iz, 2] = y1d[iy] + 0.1 * sin(π * x1d[ix]) * sin(π * z1d[iz])
    coordinates[ix, iy, iz, 3] = z1d[iz] + 0.1 * sin(π * x1d[ix]) * sin(π * y1d[iy])
end
mesh = Dict(
    :type => :curvilinear,
    :coordinates => coordinates
)

# --- Define time points ---
times = range(0, 1, length=11) |> collect
nt = length(times)

# --- Generate time-dependent data (moving 3D Gaussian) ---
data = Array{Float64}(undef, nx, ny, nz, nt)
for (ti, t) in enumerate(times)
    xc = 0.5 + 0.2 * sin(2π * t)
    yc = 0.5 + 0.2 * cos(2π * t)
    zc = 0.5
    for ix in 1:nx, iy in 1:ny, iz in 1:nz
        xi = coordinates[ix, iy, iz, 1]
        yi = coordinates[ix, iy, iz, 2]
        zi = coordinates[ix, iy, iz, 3]
        data[ix, iy, iz, ti] = exp(-((xi - xc)^2 + (yi - yc)^2 + (zi - zc)^2)/0.02)
    end
end

# --- Write mesh and data to separate HDF5 files ---
write_mesh("mesh3d.h5", mesh)
write_data("data3d.h5", data; name="solution")

# --- Link mesh and data with XDMF, including time list ---
write_xdmf("mesh3d.xdmf", "data3d.h5", "mesh3d.h5", mesh, data_name="solution", times=times)

println("Mesh written to mesh3d.h5, data to data3d.h5, XDMF to mesh3d.xdmf")