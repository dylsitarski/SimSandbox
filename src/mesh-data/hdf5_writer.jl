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
            file["coords"] = mesh[:coords]  # coords: (Nx, Ny, Nz, 3) array
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
