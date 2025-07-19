using HDF5

"""
    write_mesh(filename, mesh)

Write mesh coordinates (and optionally connectivity) to an HDF5 file.
- mesh: Dict or struct with keys :type, :x, :y, :z, and optionally :points, :connectivity, etc.
"""
function write_mesh(filename::String, mesh)
    h5open(filename, "w") do file
        if mesh[:type] == :rectilinear
            file["x"] = mesh[:x]
            file["y"] = mesh[:y]
            file["z"] = mesh[:z]
        elseif mesh[:type] == :structured
            # Recommend: treat as unstructured from the start
            error("Structured mesh writing is not supported. Please use rectilinear (x, y, z vectors) for regular grids or unstructured (points, connectivity) for arbitrary coordinates.")
        elseif mesh[:type] == :unstructured
            file["points"] = mesh[:points]  # (Npoints, 3)
            file["connectivity"] = mesh[:connectivity]  # (Nelements, nodes_per_element)
        else
            error("Unknown mesh type: $(mesh[:type])")
        end
    end
end

"""
    write_data(filename, data; name="solution", times=nothing)

Write solution data to an HDF5 file under the given dataset name.
- data: Array (shape must match mesh for XDMF linking)
- times: Optional vector of time values (for time-dependent data)
"""
function write_data(filename::String, data; name="solution")
    h5open(filename, "w") do file
        file[name] = data
    end
end

"""
    read_mesh(filename)

Read mesh coordinates (and optionally connectivity) from an HDF5 file.
Returns a Dict with keys :type, :x, :y, :z, :coords, :points, :connectivity as available.
"""
function read_mesh(filename::String)
    h5open(filename, "r") do file
        mesh = Dict{Symbol, Any}()
        # Try to detect mesh type by available datasets
        if haskey(file, "x") && haskey(file, "y") && haskey(file, "z")
            mesh[:type] = :rectilinear
            mesh[:x] = read(file["x"])
            mesh[:y] = read(file["y"])
            mesh[:z] = read(file["z"])
        elseif haskey(file, "points") && haskey(file, "connectivity")
            mesh[:type] = :unstructured
            mesh[:points] = read(file["points"])
            mesh[:connectivity] = read(file["connectivity"])
        else
            error("Unknown mesh format in file: $filename")
        end
        return mesh
    end
end
