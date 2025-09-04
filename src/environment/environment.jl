module Environment

export Zone, Mesh, Field, BC, State, generate_structured_mesh

# ----------------------------
# Core definitions
# ----------------------------
struct Mesh
    points::Matrix{Float64}                 # size (3, Nnodes), columns are (x,y,z)
    cells::Vector{Vector{Int}}              # list of per-cell connectivity vectors (1-based node indices)
    cell_types::Vector{Int}                 # VTK cell types per cell (e.g., 12 = hexahedron)
    point_regions::Dict{String,Vector{Int}}  # region name -> list of point indices
    cell_regions::Dict{String,Vector{Int}}   # region name -> list of cell indices
end

struct Zone
    mesh_id::String   # identifier for the mesh this zone is associated with
    basis::Symbol      # :point or :cell
    type::Symbol      # :static | :predicate | :parametric | :levelset
    data::Any         # payload depends on type
end

# Lightweight Field and BC types for simulation state
struct Field{T}
    mesh_id::String   # identifier for the mesh this field is associated with
    basis::Symbol      # :point or :cell
    data::Vector{T}               # per-point or per-cell values
    time::Union{Nothing, Float64} # optional timestamp
end

struct BC
    mesh_id::String   # identifier for the mesh this BC is associated with
    zone::String      # zone name (matches State.zones)
    condition::Function    # function providing BC values (usually implemented in src/bcs)
end

# State aggregates a Mesh with simulation fields, BCs and zone descriptors
struct State
    meshes::Dict{String,Mesh}   # name -> Mesh
    fields::Dict{String,Field}         # name -> Field
    bcs::Dict{String,BC}             # name -> BC
    zones::Dict{String,Zone}     # name -> Zone
end

# When constructing a State from a Mesh, automatically inherit mesh regions as static zones
function State(meshes::Dict{String,Mesh}; fields=Dict{String,Field}(), bcs=Dict{String,BC}(), zones=nothing)
    # start with regions from mesh, converted to Zone descriptors (type=:static)
    inherited = Dict{String,Zone}()
    for (mesh_id, mesh) in meshes
        for (name, inds) in mesh.point_regions
            inherited[name] = Zone(mesh_id, :point, :static, deepcopy(inds))
        end
        for (name, inds) in mesh.cell_regions
            inherited[name] = Zone(mesh_id, :cell, :static, deepcopy(inds))
        end
    end
    # merge user-supplied zones (allow overriding or adding dynamic zones)
    if zones !== nothing
        for (k,v) in zones
            inherited[k] = v
        end
    end
    return State(meshes, fields, bcs, inherited)
end

# ----------------------------
# Structured mesh generator TODO: temporary, replace with proper meshing library
# ----------------------------
function generate_structured_mesh(nx, ny, nz; lx=1.0, ly=1.0, lz=1.0)
    dx, dy, dz = lx/nx, ly/ny, lz/nz
    # nodes
    xs = 0:dx:lx
    ys = 0:dy:ly
    zs = 0:dz:lz
    nodes = Float64[]
    node_index = Dict{Tuple{Int,Int,Int},Int}()
    idx = 1
    for k in 1:length(zs), j in 1:length(ys), i in 1:length(xs)
        push!(nodes, xs[i]); push!(nodes, ys[j]); push!(nodes, zs[k])
        node_index[(i,j,k)] = idx
        idx += 1
    end
    nodes = reshape(nodes, 3, :)

    # cells (hexahedra, VTK cell type 12)
    cells = Vector{Vector{Int}}()
    for k in 1:nz, j in 1:ny, i in 1:nx
        n000 = node_index[(i, j, k)]
        n100 = node_index[(i+1, j, k)]
        n110 = node_index[(i+1, j+1, k)]
        n010 = node_index[(i, j+1, k)]
        n001 = node_index[(i, j, k+1)]
        n101 = node_index[(i+1, j, k+1)]
        n111 = node_index[(i+1, j+1, k+1)]
        n011 = node_index[(i, j+1, k+1)]
        push!(cells, [n000, n100, n110, n010, n001, n101, n111, n011])
    end

    # canonical cell types vector; VTK hexahedron id = 12
    cell_types = fill(12, length(cells))

    # build simple point zones on the six bounding faces
    xmin_pts = Int[]; xmax_pts = Int[]
    ymin_pts = Int[]; ymax_pts = Int[]
    zmin_pts = Int[]; zmax_pts = Int[]
    for k in 1:length(zs), j in 1:length(ys), i in 1:length(xs)
        idxn = node_index[(i,j,k)]
        if i == 1 push!(xmin_pts, idxn) end
        if i == length(xs) push!(xmax_pts, idxn) end
        if j == 1 push!(ymin_pts, idxn) end
        if j == length(ys) push!(ymax_pts, idxn) end
        if k == 1 push!(zmin_pts, idxn) end
        if k == length(zs) push!(zmax_pts, idxn) end
    end
    point_zones = Dict("xmin"=>xmin_pts, "xmax"=>xmax_pts,
                       "ymin"=>ymin_pts, "ymax"=>ymax_pts,
                       "zmin"=>zmin_pts, "zmax"=>zmax_pts)

    # simple cell zone listing all cells
    cell_zones = Dict("all" => collect(1:length(cells)))

    Mesh(nodes, cells, cell_types, point_zones, cell_zones)
end

end # module
