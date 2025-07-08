module Utils

using HDF5

export SolutionWriteHandler, initWriteHandler, closeWriteHandler

include("write_handler.jl")

end