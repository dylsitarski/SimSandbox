"""
    write_xdmf(xdmf_filename, data_filename, mesh_filename, mesh; data_name="solution", times=nothing)

Automatically generate an XDMF file for a mesh and data stored in HDF5, inferring topology and geometry type.
- mesh: Dict or struct with keys :type (Symbol), and depending on type: :coordinates (4D array for curvilinear) or :points, :connectivity, :topology (for unstructured)
- Supported types: :curvilinear, :unstructured
- times: optional list of time values to include in the XDMF file
"""
function write_xdmf(xdmf_filename::String, data_filename::String, mesh_filename::String, mesh; data_name="solution", times=nothing)
    mesh_type = mesh[:type]
    attr_center = (mesh_type == :curvilinear ? "Node" : "Cell")
    if mesh_type == :curvilinear
        topology = "3DSMesh"
        geometry = "XYZ"
        i, j, k = size(mesh[:coordinates])[1:3]
        dims = "$i $j $k"
        attr_dims = "$i $j $k"
        topology_attr = "Dimensions=\"$dims\""
        geom_xml = """
          <DataItem Dimensions="$dims 3" NumberType="Float" Precision="8" Format="HDF">$mesh_filename:/Geometry/Coordinates</DataItem>
        """
        conn_xml = ""
    elseif mesh_type == :unstructured
        topology = mesh[:topology]  # e.g., "Hexahedron", "Tetrahedron", etc.
        geometry = "XYZ"
        num_points = size(mesh[:points], 1)
        num_elements = size(mesh[:connectivity], 1)
        nodes_per_element = size(mesh[:connectivity], 2)
        attr_dims = "$num_elements"
        topology_attr = "NumberOfElements=\"$num_elements\""
        geom_xml = """
          <DataItem Dimensions="$num_points 3" NumberType="Float" Precision="8" Format="HDF">$mesh_filename:/Geometry/points</DataItem>
        """
        conn_xml = """
          <DataItem Dimensions="$num_elements $nodes_per_element" NumberType="Int" Format="HDF">$mesh_filename:/Topology/connectivity</DataItem>
        """
    else
        error("Unknown mesh type: $mesh_type")
    end

    if times !== nothing && length(times) > 0
        ntime = length(times)
        grids = String[]
        for ti in 1:ntime
            time_val = times[ti]
            grid = """
      <Grid Name="mesh_t$(ti-1)" GridType="Uniform">
        <Time Value="$time_val" />
        <Topology TopologyType="$topology" $topology_attr>
$conn_xml
        </Topology>
        <Geometry GeometryType="$geometry">
$geom_xml
        </Geometry>
        <Attribute Name="$data_name" AttributeType="Scalar" Center="$attr_center">
          <DataItem Dimensions="$attr_dims" NumberType="Float" Precision="8" Format="HDF">$data_filename:/$data_name"_"$(ti-1)</DataItem>
        </Attribute>
      </Grid>
"""
            push!(grids, grid)
        end
        xdmf = """
<?xml version="1.0" ?>
<Xdmf Version="3.0">
  <Domain>
    <Grid Name="TimeSeries" GridType="Collection" CollectionType="Temporal">
$(join(grids, "\n"))
    </Grid>
  </Domain>
</Xdmf>
"""
    else
        # Static (single time) case
        xdmf = """
<?xml version="1.0" ?>
<Xdmf Version="3.0">
  <Domain>
    <Grid Name="mesh" GridType="Uniform">
      <Topology TopologyType="$topology" $topology_attr>
$conn_xml
      </Topology>
      <Geometry GeometryType="$geometry">
$geom_xml
      </Geometry>
      <Attribute Name="$data_name" AttributeType="Scalar" Center="$attr_center">
        <DataItem Dimensions="$attr_dims" NumberType="Float" Precision="8" Format="HDF">$data_filename:/$data_name</DataItem>
      </Attribute>
    </Grid>
  </Domain>
</Xdmf>
"""
    end
    open(xdmf_filename, "w") do io
        write(io, xdmf)
    end
end