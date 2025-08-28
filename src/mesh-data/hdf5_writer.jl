using HDF5

"""
    write_mesh(filename, mesh)

Write mesh coordinates (and optionally connectivity) to an HDF5 file.
- mesh: Dict or struct with keys :type, and depending on type: :coordinates (4D array i x j x k x 3 for curvilinear) or :points, :connectivity (for unstructured).
"""
function write_mesh(filename::String, mesh)
    h5open(filename, "w") do file
        if mesh[:type] == :curvilinear
            coordinates = mesh[:coordinates]  # i x j x k x 3
            i, j, k = size(coordinates)[1:3]
            coordinates_flat = reshape(coordinates, (i*j*k, 3))  # {npoints, 3}
            coordinates_flat_transposed = permutedims(coordinates_flat, (2, 1))  # {3, npoints}
            file["/Geometry/Coordinates"] = coordinates_flat_transposed  # Stored as {npoints, 3}
            file["/Geometry/Dimensions"] = [i, j, k]
        elseif mesh[:type] == :unstructured
            file["/Geometry/points"] = mesh[:points]  # (Npoints, 3)
            file["/Topology/connectivity"] = mesh[:connectivity]  # (Nelements, nodes_per_element)
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
function write_data(filename::String, data; name="solution", times=nothing)
    h5open(filename, "w") do file
        if times === nothing
            file["/" * name] = data
        else
            nt = length(times)
            nd = ndims(data)
            @assert size(data, nd) == nt "Time dimension must match length of times"
            for ti in 1:nt
                data_slice = data[(Colon() for _ in 1:nd-1)..., ti]
                file["/" * name * "_$(ti-1)"] = data_slice
            end
        end
    end
end

"""
    read_mesh(filename)

Read mesh coordinates (and optionally connectivity) from an HDF5 file.
Returns a Dict with keys :type, and depending on type: :coordinates (for curvilinear) or :points, :connectivity (for unstructured).
"""
function read_mesh(filename::String)
    h5open(filename, "r") do file
        mesh = Dict{Symbol, Any}()
        # Try to detect mesh type by available datasets
        if haskey(file, "/Geometry/Coordinates")
            mesh[:type] = :curvilinear
            coords_flat_transposed = read(file["/Geometry/Coordinates"])
            coords_flat = permutedims(coords_flat_transposed, (2, 1))
            if haskey(file, "/Geometry/Dimensions")
                dims = read(file["/Geometry/Dimensions"])
                mesh[:coordinates] = reshape(coords_flat, (dims[1], dims[2], dims[3], 3))
            else
                error("Dimensions not found in curvilinear mesh")
            end
        elseif haskey(file, "/Geometry/points") && haskey(file, "/Topology/connectivity")
            mesh[:type] = :unstructured
            mesh[:points] = read(file["/Geometry/points"])
            mesh[:connectivity] = read(file["/Topology/connectivity"])
        else
            error("Unknown mesh format in file: $filename")
        end
        return mesh
    end
end