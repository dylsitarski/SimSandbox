"""
    write_xdmf(xdmf_filename, h5_filename, mesh; data_name="solution", times=nothing)

Automatically generate an XDMF file for a mesh and data stored in HDF5, inferring topology and geometry type.
- mesh: Dict or struct with keys :type (Symbol), :x, :y, :z (Vectors), and optionally :connectivity (for unstructured)
- Supported types: :rectilinear, :unstructured
- times: optional list of time values to include in the XDMF file
"""
function write_xdmf(xdmf_filename::String, data_filename::String, mesh_filename::String, mesh; data_name="solution", times=nothing)
    mesh_type = mesh[:type]
    if mesh_type == :rectilinear
        topology = "3DRectMesh"
        geometry = "VXVYVZ"
        dims = "$(length(mesh[:z])) $(length(mesh[:y])) $(length(mesh[:x]))"
        geom_xml = """
        <DataItem Dimensions=\"$(length(mesh[:x]))\" NumberType=\"Float\" Precision=\"8\" Format=\"HDF\">$mesh_filename:/x</DataItem>
        <DataItem Dimensions=\"$(length(mesh[:y]))\" NumberType=\"Float\" Precision=\"8\" Format=\"HDF\">$mesh_filename:/y</DataItem>
        <DataItem Dimensions=\"$(length(mesh[:z]))\" NumberType=\"Float\" Precision=\"8\" Format=\"HDF\">$mesh_filename:/z</DataItem>
        """
    elseif mesh_type == :structured
        # Recommend: treat as unstructured from the start
        error("Structured mesh writing is not supported. Please use rectilinear (x, y, z vectors) for regular grids or unstructured (points, connectivity) for arbitrary coordinates.")
    elseif mesh_type == :unstructured
        topology = mesh[:topology]  # e.g., "Hexahedron", "Tetrahedron", etc.
        geometry = "XYZ"
        dims = string(mesh[:num_elements], " ", mesh[:nodes_per_element])
        geom_xml = """
        <DataItem Dimensions=\"$(mesh[:num_points]) 3\" NumberType=\"Float\" Precision=\"8\" Format=\"HDF\">$mesh_filename:/points</DataItem>
        """
        conn_xml = """
        <DataItem Dimensions=\"$dims\" NumberType=\"Int" Format=\"HDF\">$mesh_filename:/connectivity</DataItem>
        """
    else
        error("Unknown mesh type: $mesh_type")
    end

    if times !== nothing && length(times) > 0
        ntime = length(times)
        time_xml = "  <Time TimeType=\"List\" NumberOfTimes=\"$ntime\">\n    <DataItem Dimensions=\"$ntime\" NumberType=\"Float\" Precision=\"8\" Format=\"XML\">$(join(times, " "))</DataItem>\n  </Time>"
        grids = String[]
        for ti in 1:ntime
            if mesh_type == :rectilinear
                nx, ny, nz = length(mesh[:x]), length(mesh[:y]), length(mesh[:z])
                grid = string(
                    "    <Grid Name=\"mesh_t", ti, "\" GridType=\"Uniform\">\n",
                    "      <Topology TopologyType=\"$topology\" Dimensions=\"$dims\"/>",
                    "      <Geometry GeometryType=\"$geometry\">\n",
                    geom_xml, "\n",
                    "      </Geometry>\n",
                    "      <Attribute Name=\"$data_name\" AttributeType=\"Scalar\" Center=\"Node\">\n",
                    "        <DataItem ItemType=\"HyperSlab\" Dimensions=\"$dims\" NumberType=\"Float\" Precision=\"8\" Format=\"HDF\">\n",
                    "          <DataItem Dimensions=\"3 4\" Format=\"XML\">\n",
                    "            ", ti-1, " 0 0 0\n",
                    "            1 1 1 1\n",
                    "            1 ", nz, " ", ny, " ", nx, "\n",
                    "          </DataItem>\n",
                    "          <DataItem Dimensions=\"$ntime ", nz, " ", ny, " ", nx, "\" NumberType=\"Float\" Precision=\"8\" Format=\"HDF\">$data_filename:/$data_name</DataItem>\n",
                    "        </DataItem>\n",
                    "      </Attribute>\n",
                    "    </Grid>\n"
                )
            elseif mesh_type == :unstructured
                nx = mesh[:num_points]
                grid = string(
                    "    <Grid Name=\"mesh_t", ti, "\" GridType=\"Uniform\">\n",
                    "      <Topology TopologyType=\"$topology\" Dimensions=\"$dims\">\n",
                    conn_xml, "\n      </Topology>\n",
                    "      <Geometry GeometryType=\"$geometry\">\n",
                    geom_xml, "\n",
                    "      </Geometry>\n",
                    "      <Attribute Name=\"$data_name\" AttributeType=\"Scalar\" Center=\"Node\">\n",
                    "        <DataItem ItemType=\"HyperSlab\" Dimensions=\"", nx, "\" NumberType=\"Float\" Precision=\"8\" Format=\"HDF\">\n",
                    "          <DataItem Dimensions=\"3 2\" Format=\"XML\">\n",
                    "            ", ti-1, " 0\n",
                    "            1 1\n",
                    "            1 ", nx, "\n",
                    "          </DataItem>\n",
                    "          <DataItem Dimensions=\"$ntime ", nx, "\" NumberType=\"Float\" Precision=\"8\" Format=\"HDF\">$data_filename:/$data_name</DataItem>\n",
                    "        </DataItem>\n",
                    "      </Attribute>\n",
                    "    </Grid>\n"
                )
            end
            push!(grids, grid)
        end
        xdmf = string(
            "<?xml version=\"1.0\" ?>\n",
            "<Xdmf Version=\"3.0\">\n",
            "  <Domain>\n",
            "    <Grid Name=\"TimeSeries\" GridType=\"Collection\" CollectionType=\"Temporal\">\n",
            time_xml, "\n",
            join(grids, "\n"),
            "    </Grid>\n",
            "  </Domain>\n",
            "</Xdmf>"
        )
    else
        # Static (single time) case
        xdmf = string(
            "<?xml version=\"1.0\" ?>\n",
            "<Xdmf Version=\"3.0\">\n",
            "  <Domain>\n",
            "    <Grid Name=\"mesh\" GridType=\"Uniform\">\n",
            "      <Topology TopologyType=\"$topology\" Dimensions=\"$dims\"/>\n",
            "      <Geometry GeometryType=\"$geometry\">\n",
            geom_xml, "\n",
            "      </Geometry>\n",
            "      <Attribute Name=\"$data_name\" AttributeType=\"Scalar\" Center=\"Node\">\n",
            "        <DataItem Dimensions=\"$dims\" NumberType=\"Float\" Precision=\"8\" Format=\"HDF\">$data_filename:/$data_name</DataItem>\n",
            "      </Attribute>\n",
            "    </Grid>\n",
            "  </Domain>\n",
            "</Xdmf>"
        )
    end
    open(xdmf_filename, "w") do io
        write(io, xdmf)
    end
end
