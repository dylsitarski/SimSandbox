"""
    SolutionWriteHandler(t, t_write, z, y, x; outpath="solution.h5", dataset="solution")

Creates a callable object that, when called as (n, tval, u), appends a 3D solution to an HDF5 file at the specified write times.
- t: time grid
- t_write: times to write
- z, y, x: spatial grids (for shape)
- outpath: output HDF5 file path
- dataset: dataset name in HDF5 file
"""
mutable struct SolutionWriteHandler
    idxs::Vector{Int}
    ptr::Int
    file::HDF5.File
    dset::HDF5.Dataset
    nx::Int
    ny::Int
    nz::Int
    ntime::Int
    tvals::Vector{Float64}
end

function initWriteHandler(t, t_write, x, y, z; outpath="solution.h5", dataset="solution")
    idxs = [findfirst(xi -> abs(xi - tw) < 1e-8, t) for tw in t_write]
    nz, ny, nx = length(z), length(y), length(x)
    file = h5open(outpath, "w")
    dset = create_dataset(file, dataset, datatype(Float64), dataspace((nx, ny, nz, 0); max_dims=(nx, ny, nz, -1)), chunk=(nx, ny, nz, 1))
    return SolutionWriteHandler(idxs, 1, file, dset, nx, ny, nz, 0, Float64[])
end

function (wh::SolutionWriteHandler)(n, tval, u)
    if wh.ptr <= length(wh.idxs) && n == wh.idxs[wh.ptr]
        wh.ntime += 1
        HDF5.h5d_extend(wh.dset, [wh.nx, wh.nz, wh.ny, wh.ntime])
        wh.dset[:, :, :, wh.ntime] = u
        push!(wh.tvals, tval)
        wh.ptr += 1
    end
end

function closeWriteHandler(wh::SolutionWriteHandler)
    close(wh.dset)
    close(wh.file)
end
