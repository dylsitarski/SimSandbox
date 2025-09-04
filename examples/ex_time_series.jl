using SimSandbox
using SimSandbox.Meshing

# Generate base mesh (structured cube, 2×2×2 cells)
mesh = generate_structured_mesh(2, 2, 2)

# We'll simulate 5 timesteps with evolving scalar field
nt = 5
timesteps = Float64[]
files = String[]

for step in 1:nt
    time = (step - 1) * 0.1
    values = [sin(time + i) for (i, _) in enumerate(mesh.cells)]
    filename = "example_mesh_$step.vtu"
    write_vtu(filename, mesh; cell_data=Dict("ScalarField" => values))
    push!(timesteps, time)
    push!(files, filename)
end

# Write master .pvd file
write_pvd("example_series.pvd", timesteps, files)

println("Wrote example_series.pvd — open in ParaView for a time animation!")
