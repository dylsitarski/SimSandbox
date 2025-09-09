module Environment

export Zone, Mesh, Field, BC, State, generate_structured_mesh

# ----------------------------
# Core definitions
# ----------------------------
struct Mesh
    points::Matrix{Float64}                 # size (3, Nnodes), columns are (x,y,z)
    cells::Vector{Vector{Int}}              # list of per-cell connectivity vectors (1-based node indices)
    point_regions::Dict{String,Vector{Int}}  # region name -> list of point indices
    cell_regions::Dict{String,Vector{Int}}   # region name -> list of cell indices
    cell_types::Union{Vector{Int}, Nothing}  # VTK cell types per cell TODO: may not be needed, just infer from connectivity length
    function Mesh(points, cells, point_regions, cell_regions; cell_types=nothing)
        new(points, cells, point_regions, cell_regions, cell_types)
    end
end

struct Zone
    mesh_id::String   # identifier for the mesh this zone is associated with
    basis::Symbol      # :point or :cell
    type::Symbol      # :static | :predicate | :parametric | :levelset
    data::Any         # payload depends on type
end

# Lightweight Field and BC types for simulation state
mutable struct Field{T}
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

end # module
