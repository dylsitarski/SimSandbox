# VTK I/O utilities (included in the FileIO module)
# TODO: add pvd writer
# TODO: add zone support
# TODO: writer needs to take State as input, reader needs to return State

using WriteVTK

# Detect whether ReadVTK is available at module load time so we can use it as a fallback
const HAS_READVTK = try
    import ReadVTK
    true
catch
    false
end

# Unified DataArray regex (module-level so it can be const)
const DATAARRAY_RE = Regex("(?s)<DataArray([^>]*)(?:>(.*?)</DataArray>|\\s*/>)")

export read_vtk, write_vtu

"""
    write_vtu(filename, points, cells; cell_types=nothing, point_data=Dict(), cell_data=Dict())

Write an unstructured VTK (.vtu) file using WriteVTK.jl.

Arguments
- `filename`: output filename (WriteVTK will add extension if omitted)
- `points`: 3×N matrix (columns are x,y,z coordinates)
- `cells`: Vector of connectivity vectors (1-based indices)
- `cell_types` (optional): Vector of VTK cell type constants matching `cells`
- `point_data`, `cell_data`: Dict{String,AbstractVector} of datasets
"""
function write_vtu(filename::AbstractString,
                   points::AbstractMatrix{<:Real},
                   cells::Vector{<:AbstractVector};
                   cell_types=nothing,
                   point_data::Dict{String,Any}=Dict(),
                   cell_data::Dict{String,Any}=Dict())

    # infer a simple VTK cell type when none provided
    infer_type(conn) = begin
        l = length(conn)
        if l == 8
            VTKCellTypes.VTK_HEXAHEDRON
        elseif l == 6
            VTKCellTypes.VTK_WEDGE
        elseif l == 5
            VTKCellTypes.VTK_PYRAMID
        elseif l == 4
            VTKCellTypes.VTK_QUAD
        elseif l == 3
            VTKCellTypes.VTK_TRIANGLE
        elseif l == 2
            VTKCellTypes.VTK_LINE
        elseif l == 1
            PolyData.Verts()
        else
            # fallback to poly vertex for arbitrary connectivity
            VTKCellTypes.VTK_POLY_VERTEX
        end
    end

    meshcells = WriteVTK.MeshCell[]
    if cell_types === nothing
        for c in cells
            push!(meshcells, WriteVTK.MeshCell(infer_type(c), collect(c)))
        end
    else
        @assert length(cell_types) == length(cells) "cell_types must match number of cells"
        for (ct_raw, c) in zip(cell_types, cells)
            ct = ct_raw
            # If the provided type is an integer code, try to convert to VTKCellTypes value
            if isa(ct_raw, Integer)
                try
                    # try constructing VTKCellType from integer code
                    ct = WriteVTK.VTKCellTypes.VTKCellType(ct_raw)
                catch
                    # fallback: try to map a few common codes to the VTK constants
                    if ct_raw == VTKCellTypes.VTK_HEXAHEDRON
                        ct = VTKCellTypes.VTK_HEXAHEDRON
                    elseif ct_raw == VTKCellTypes.VTK_TETRA
                        ct = VTKCellTypes.VTK_TETRA
                    elseif ct_raw == VTKCellTypes.VTK_QUAD
                        ct = VTKCellTypes.VTK_QUAD
                    elseif ct_raw == VTKCellTypes.VTK_TRIANGLE
                        ct = VTKCellTypes.VTK_TRIANGLE
                    elseif ct_raw == VTKCellTypes.VTK_LINE
                        ct = VTKCellTypes.VTK_LINE
                    else
                        # cannot convert; infer from connectivity
                        @warn "Unknown integer VTK cell type $ct_raw; falling back to inference based on connectivity length"
                        ct = infer_type(c)
                    end
                end
            end
            push!(meshcells, WriteVTK.MeshCell(ct, collect(c)))
        end
    end

    # Use WriteVTK to write the file. WriteVTK will pick extension based on dataset type.
    WriteVTK.vtk_grid(filename, points, meshcells) do vtk
        for (name, vals) in point_data
            vtk[name] = vals
        end
        for (name, vals) in cell_data
            vtk[name] = vals
        end
    end

    return nothing
end

"""
    read_vtk(filename)

Read a VTK XML file and extract points, cells and dataset arrays.

Lightweight ASCII XML parser that handles DataArray blocks.
"""
function read_vtk(filename::AbstractString)
    # Read file as raw bytes to support appended binary blocks
    raw = read(filename)

    # Optional ReadVTK fallback support (use when our lightweight parser fails)
    # const HAS_READVTK = try
    #     @eval using ReadVTK
    #     true
    # catch
    #     false
    # end

    appended_payload = nothing
    # Locate AppendedData block (if present) without converting entire file to String
    open_marker = collect(codeunits("<AppendedData"))
    close_marker = collect(codeunits("</AppendedData>"))
    pos_open = findfirst(open_marker, raw)

    if pos_open !== nothing
        # find end of opening tag '>' by searching the subarray from pos_open
        if isa(pos_open, UnitRange)
            pos_open = first(pos_open)
        end
        rel_gt = findfirst(x -> x == UInt8('>'), raw[pos_open:end])
        rel_gt = isa(rel_gt, UnitRange) ? first(rel_gt) : rel_gt
        gtpos = rel_gt === nothing ? nothing : (pos_open - 1 + rel_gt)
        rel_close = findfirst(close_marker, raw[pos_open:end])
        rel_close = isa(rel_close, UnitRange) ? first(rel_close) : rel_close
        pos_close = rel_close === nothing ? nothing : (pos_open - 1 + rel_close)
        if gtpos === nothing || pos_close === nothing
            error("Malformed AppendedData in VTK file: $filename")
        end
        inner_start = gtpos + 1
        inner_end = pos_close - 1
        appended_block = raw[inner_start:inner_end]
        # Strip leading '_' marker and optional newline (VTK convention)
        if !isempty(appended_block) && appended_block[1] == UInt8('_')
            appended_payload = appended_block[2:end]
            if !isempty(appended_payload) && (appended_payload[1] == UInt8('\n') || appended_payload[1] == UInt8('\r'))
                appended_payload = appended_payload[2:end]
            end
        else
            appended_payload = appended_block
        end
        header_bytes = raw[1:pos_open-1]
        s = String(header_bytes)
        # detect header_type so we know whether block lengths are 32- or 64-bit
        header_type_str = "UInt32"
        mht = match(Regex("<VTKFile[^>]*header_type=\\\"(.*?)\\\""), s)
        if mht !== nothing
            header_type_str = mht.captures[1]
        end
    else
        # no appended data — safe to convert full file to String
        s = String(raw)
    end

    # Helper: parse attributes string like Name="connectivity" NumberOfComponents="3"
    function parse_attrs(attrstr::AbstractString)
        attrs = Dict{String,String}()
        for m in eachmatch(Regex("(\\w+)\\s*=\\s*\"([^\"]*)\""), attrstr)
            attrs[m.captures[1]] = m.captures[2]
        end
        return attrs
    end

    # Helper: parse a DataArray element match (attrstr, content)
    function parse_dataarray(attrstr::AbstractString, content::AbstractString)
        attrs = parse_attrs(attrstr)

        # If this DataArray uses appended binary data, read from appended_payload by offset
        if haskey(attrs, "offset") && appended_payload !== nothing
            off = parse(Int, attrs["offset"])  # offset in bytes from start of appended_payload
            io = IOBuffer(appended_payload)
            # seek uses 1-based indexing; set position to off+1
            seek(io, off + 1)
            # first 4 bytes (UInt32) give the block length
            len = read(io, UInt32)
            # read block length using header_type detected from file
            if header_type_str == "UInt64"
                lenu = read(io, UInt64)
            else
                lenu = read(io, UInt32)
            end
            len = Int(lenu)
            # read the block bytes
            datab = read(io, Int(len))
            buf = IOBuffer(datab)

            # determine element type from attrs["type"] (VTK type names)
            vtype = get(attrs, "type", "Float64")
            eltype = Float64
            if vtype == "Float64"
                eltype = Float64
            elseif vtype == "Float32"
                eltype = Float32
            elseif vtype == "Int32"
                eltype = Int32
            elseif vtype == "Int64"
                eltype = Int64
            elseif vtype == "UInt8"
                eltype = UInt8
            else
                # default to Float64
                eltype = Float64
            end

            total_elems = Int(len) ÷ sizeof(eltype)
            # reinterpret raw bytes as elements of eltype
            vals_raw = collect(reinterpret(eltype, datab))[1:total_elems]

            # handle NumberOfComponents
            nc = haskey(attrs, "NumberOfComponents") ? parse(Int, attrs["NumberOfComponents"]) : 1
            if nc > 1
                npoints = div(total_elems, nc)
                mat = reshape(Float64.(vals_raw), nc, npoints)
                return (attrs, mat)
            else
                # return vector of appropriate eltype (convert small ints to Int)
                if eltype <: Integer
                    return (attrs, Int.(vals_raw))
                else
                    return (attrs, Float64.(vals_raw))
                end
            end
        end

        # Fallback: inline ASCII content
        txt = strip(replace(content, '\n' => ' '))
        toks = [t for t in split(txt) if !isempty(t)]
        # Try parsing as Int where possible, else Float64
        vals_int = Int[]
        vals_float = Float64[]
        got_float = false
        for t in toks
            try
                push!(vals_int, parse(Int, t))
            catch
                push!(vals_float, parse(Float64, t))
                got_float = true
            end
        end
        vals = got_float ? vals_float : vals_int
        # If NumberOfComponents present and >1, reshape into components × N array
        if haskey(attrs, "NumberOfComponents")
            nc = tryparse(Int, attrs["NumberOfComponents"]) === nothing ? 1 : parse(Int, attrs["NumberOfComponents"]) 
            if nc > 1 && length(vals) % nc == 0
                npoints = length(vals) ÷ nc
                if got_float
                    mat = reshape(Float64.(vals), nc, npoints)
                    return (attrs, mat)
                else
                    mat = reshape(Int.(vals), nc, npoints)
                    return (attrs, mat)
                end
            end
        end
        return (attrs, vals)
    end

    # If ReadVTK is available and file looks like it was written with WriteVTK (compressed/appended/header_type),
    # prefer ReadVTK which already handles appended/compressed layout.
    header_sig = s
    if HAS_READVTK && (occursin("vtkZLibDataCompressor", header_sig) || occursin("header_type=\"UInt", header_sig) || occursin("format=\"appended\"", header_sig))
        try
            vf = ReadVTK.VTKFile(filename)
            pts = ReadVTK.get_points(vf)
            vtkcells = ReadVTK.get_cells(vf)
            meshcells = ReadVTK.to_meshcells(vtkcells)

            conn = Int[]
            offs = Int[]
            types_out = Int[]
            offacc = 0
            cell_lists = Vector{Vector{Int}}()
            for mc in meshcells
                # mc may be a MeshCell-like object; try common access patterns
                conn_local = try
                    collect(mc.connectivity)
                catch
                    try
                        collect(mc[:connectivity])
                    catch
                        []
                    end
                end
                append!(conn, Int.(conn_local))
                offacc += length(conn_local)
                push!(offs, offacc)
                push!(cell_lists, Int.(conn_local))
                # Extract VTK cell type id: prefer `:ctype` (WriteVTK MeshCell field), fallback to `:cell_type`.
                ct = nothing
                if hasproperty(mc, :ctype)
                    cfield = getproperty(mc, :ctype)
                    try
                        ct = cfield.vtk_id
                    catch
                        ct = cfield
                    end
                elseif hasproperty(mc, :cell_type)
                    cfield = getproperty(mc, :cell_type)
                    try
                        ct = cfield.vtk_id
                    catch
                        ct = cfield
                    end
                end
                if ct === nothing
                    push!(types_out, 0)
                else
                    push!(types_out, Int(ct))
                end
            end

            pd = ReadVTK.get_point_data(vf)
            cd = ReadVTK.get_cell_data(vf)
            pdm = Dict{String,Any}()
            for (k,v) in pd
                # try to extract raw data array using ReadVTK helpers
                val = try
                    ReadVTK.get_data(v)
                catch
                    try
                        ReadVTK.get_data_reshaped(v)
                    catch
                        v
                    end
                end
                # ensure plain Array/Vector
                try
                    pdm[string(k)] = Array(val)
                catch
                    pdm[string(k)] = val
                end
            end
            cdm = Dict{String,Any}()
            for (k,v) in cd
                val = try
                    ReadVTK.get_data(v)
                catch
                    try
                        ReadVTK.get_data_reshaped(v)
                    catch
                        v
                    end
                end
                try
                    cdm[string(k)] = Array(val)
                catch
                    cdm[string(k)] = val
                end
            end

            return Dict(
                :points => pts,
                :connectivity => conn,
                :offsets => offs,
                :cell_types => types_out,
                :cells => cell_lists,
                :point_data => pdm,
                :cell_data => cdm,
            )
        catch e
            @warn "ReadVTK fast-path failed, falling back to lightweight parser: $e"
        end
    end

    # Try parsing with our lightweight parser; if anything goes wrong and ReadVTK is available,
    # fall back to ReadVTK which robustly handles files produced by WriteVTK.
    parsed_success = false
    try
        # Extract Points: find first DataArray inside <Points> (supports appended/self-closing)
        mp = match(Regex("(?s)<Points>(.*?)</Points>"), s)
        if mp === nothing
            error("Could not find Points block in $filename")
        end
        points_block = mp.captures[1]
        m = match(DATAARRAY_RE, points_block)
        if m === nothing
            error("Could not find DataArray inside Points in $filename")
        end
        _, ptsvals = parse_dataarray(m.captures[1], m.captures[2] === nothing ? "" : m.captures[2])

        # ptsvals may be a matrix components×N or a flat vector; ensure Float64 matrix 3×N
        if isa(ptsvals, AbstractMatrix)
            points = Float64.(ptsvals)
            if size(points, 1) != 3 && size(points, 2) == 3
                points = permutedims(points)
            end
        else
            nums = Float64.(ptsvals)
            @assert length(nums) % 3 == 0 "Points data length not divisible by 3"
            npts = length(nums) ÷ 3
            points = reshape(nums, 3, npts)
        end

        # Extract all DataArray elements inside <Cells>
        cells_block = match(Regex("(?s)<Cells>(.*?)</Cells>"), s)
        connectivity = Int[]; offsets = Int[]; types = Int[]
        if cells_block !== nothing
            block = cells_block.captures[1]
            for m2 in eachmatch(DATAARRAY_RE, block)
                attrs, vals = parse_dataarray(m2.captures[1], m2.captures[2] === nothing ? "" : m2.captures[2])
                name = haskey(attrs, "Name") ? attrs["Name"] : ""
                if name == "connectivity"
                    # connectivity should be integers
                    if isa(vals, AbstractMatrix)
                        # flatten
                        vals = vec(vals)
                    end
                    connectivity = Int.(vals)
                elseif name == "offsets"
                    offsets = Int.(vals)
                elseif lowercase(name) == "types"
                    types = Int.(vals)
                end
            end
        end

        # If connectivity empty, try to find any DataArray named "connectivity" anywhere
        if isempty(connectivity)
            for m2 in eachmatch(DATAARRAY_RE, s)
                attrs = parse_attrs(m2.captures[1])
                if haskey(attrs, "Name") && attrs["Name"] == "connectivity"
                    _, vals = parse_dataarray(m2.captures[1], m2.captures[2] === nothing ? "" : m2.captures[2])
                    connectivity = Int.(isa(vals, AbstractMatrix) ? vec(vals) : vals)
                    break
                end
            end
        end

        # Heuristic: if any zero present in connectivity, assume 0-based and convert to 1-based
        if any(x -> x == 0, connectivity)
            connectivity .= connectivity .+ 1
        end

        # Build per-cell connectivity vectors using offsets. If offsets empty, try to infer cell sizes using types (not implemented)
        cells = Vector{Vector{Int}}()
        if !isempty(offsets) && !isempty(connectivity)
            prev = 0
            for off in offsets
                len = off - prev
                push!(cells, collect(connectivity[prev+1:prev+len]))
                prev = off
            end
        end

        # Extract PointData and CellData
        function extract_data_block(text, blockname)
            # match opening tag with optional attributes, e.g. <CellData Scalars="scalars">
            mb = match(Regex("(?s)<" * blockname * "[^>]*>(.*?)</" * blockname * ">"), text)
            out = Dict{String, Any}()
            if mb === nothing
                return out
            end
            block = mb.captures[1]
            for m3 in eachmatch(DATAARRAY_RE, block)
                attrs, vals = parse_dataarray(m3.captures[1], m3.captures[2] === nothing ? "" : m3.captures[2])
                name = haskey(attrs, "Name") ? attrs["Name"] : ""
                if isa(vals, AbstractMatrix)
                    out[name] = vals
                else
                    out[name] = vals
                end
            end
            return out
        end

        point_data = extract_data_block(s, "PointData")
        cell_data = extract_data_block(s, "CellData")
        parsed_success = true
        return Dict(
            :points => points,
            :connectivity => connectivity,
            :offsets => offsets,
            :cell_types => types,
            :cells => cells,
            :point_data => point_data,
            :cell_data => cell_data,
        )
    catch err
        @warn "read_vtk: lightweight parser failed: $err"
        if !HAS_READVTK
            rethrow(err)
        end
        # Fall back to ReadVTK
        try
            vf = ReadVTK.VTKFile(filename)
            pts = ReadVTK.get_points(vf)
            vtkcells = ReadVTK.get_cells(vf)
            # convert to meshcells using ReadVTK helper and extract connectivity/offsets/types
            meshcells = ReadVTK.to_meshcells(vtkcells)
            conn = Int[]; offs = Int[]; types_out = Int[]
            offacc = 0
            for mc in meshcells
                push!(conn, [collect(mc.connectivity)...]... )
                offacc += length(mc.connectivity)
                push!(offs, offacc)
                push!(types_out, ReadVTK.VTKPrimitives.cell_type(mc).vtk_id)
            end
            pd = ReadVTK.get_point_data(vf)
            cd = ReadVTK.get_cell_data(vf)
            pdm = Dict{String,Any}()
            for (k,v) in pd
                # try to extract raw data array using ReadVTK helpers
                val = try
                    ReadVTK.get_data(v)
                catch
                    try
                        ReadVTK.get_data_reshaped(v)
                    catch
                        v
                    end
                end
                # ensure plain Array/Vector
                try
                    pdm[string(k)] = Array(val)
                catch
                    pdm[string(k)] = val
                end
            end
            cdm = Dict{String,Any}()
            for (k,v) in cd
                val = try
                    ReadVTK.get_data(v)
                catch
                    try
                        ReadVTK.get_data_reshaped(v)
                    catch
                        v
                    end
                end
                try
                    cdm[string(k)] = Array(val)
                catch
                    cdm[string(k)] = val
                end
            end
            return Dict(
                :points => pts,
                :connectivity => conn,
                :offsets => offs,
                :cell_types => types_out,
                :cells => [collect(mc.connectivity) for mc in meshcells],
                :point_data => pdm,
                :cell_data => cdm,
            )
        catch e2
            @warn "ReadVTK fallback failed: $e2"
            rethrow(err)
        end
    end
end
