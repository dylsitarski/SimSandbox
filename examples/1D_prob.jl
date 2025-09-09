using WriteVTK
using SimSandbox.Environment
using SimSandbox.FileIO

# Simple 1D problem: mesh on x in [0,1]
function make_1d_mesh(nx::Int)
	xs = range(0.0, 1.0; length=nx+1)
	# points as 3xN matrix (y,z zero)
	pts = zeros(Float64, 3, length(xs))
	pts[1, :] .= collect(xs)
	# cells: each cell connects two consecutive node indices (1-based)
	cells = [ [i, i+1] for i in 1:length(xs)-1 ]
	point_regions = Dict{String,Vector{Int}}()
	cell_regions = Dict{String,Vector{Int}}()
	return Mesh(pts, cells, point_regions, cell_regions)
end

mesh = make_1d_mesh(10)

mesh.point_regions["b1"] = [1]
mesh.point_regions["b2"] = [size(mesh.points,2)]

mesh.cell_regions["left"] = collect(1:5)
mesh.cell_regions["right"] = collect(6:10)

# Build a State and attach a sample point field (e.g., x coordinate) and a cell field
st = State(Dict("default" => mesh))
st.fields["xcoord"] = Field("default", :point, vec(mesh.points[1,:]), nothing)
st.fields["pressure"] = Field("default", :cell, [1 for c in st.zones["left"].data], nothing)

st.fields["pressure"].data = cat(st.fields["pressure"].data, [0 for c in st.zones["right"].data], dims=1)

# Write VTU for inspection
write_vtu(st, "1d_mesh.vtu")

println("Wrote 1d_mesh.vtu with $(size(mesh.points,2)) points and $(length(mesh.cells)) cells")

