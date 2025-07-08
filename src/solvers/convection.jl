using SimSandbox.Utils: SolutionWriteHandler, initWriteHandler, closeWriteHandler

mutable struct Convection3DSolver
    u::Array{Float64,3}
    x::Vector{Float64}
    y::Vector{Float64}
    z::Vector{Float64}
    t::Vector{Float64}
    bcs
    timestepper
    c::NTuple{3,Float64}  # (cx, cy, cz)
    t_write::Vector{Float64}
    outpath::String
    scheme::Symbol
    write_handler
end

function Convection3DSolver(u0, x, y, z, t, bcs; timestepper, c=(1.0,1.0,1.0), t_write=[t[end]], outpath="convection3d_output.h5", scheme=:upwind)
    write_handler = initWriteHandler(t, t_write, x, y, z; outpath=outpath)
    solver = Convection3DSolver(copy(u0), x, y, z, t, bcs, timestepper, c, t_write, outpath, scheme, write_handler)
    # Write initial condition if t[1] is in t_write
    write_handler(1, t[1], solver.u)
    return solver
end

function rhs!(solver::Convection3DSolver, u)
    nx, ny, nz = length(solver.x), length(solver.y), length(solver.z)
    dudx = zeros(nx, ny, nz)
    dudy = zeros(nx, ny, nz)
    dudz = zeros(nx, ny, nz)
    # x-direction
    for k in 1:nz, j in 1:ny
        if solver.scheme == :central
            for i in 2:nx-1
                dudx[i,j,k] = (u[i+1,j,k] - u[i-1,j,k]) / (solver.x[i+1] - solver.x[i-1])
            end
            dudx[1,j,k] = (u[2,j,k] - u[end,j,k]) / (solver.x[2] - solver.x[end])
            dudx[nx,j,k] = (u[1,j,k] - u[nx-1,j,k]) / (solver.x[1] - solver.x[nx-1])
        else
            if solver.c[1] >= 0
                for i in 2:nx
                    dudx[i,j,k] = (u[i,j,k] - u[i-1,j,k]) / (solver.x[i] - solver.x[i-1])
                end
                dudx[1,j,k] = (u[1,j,k] - u[end,j,k]) / (solver.x[1] - solver.x[end])
            else
                for i in 1:nx-1
                    dudx[i,j,k] = (u[i+1,j,k] - u[i,j,k]) / (solver.x[i+1] - solver.x[i])
                end
                dudx[nx,j,k] = (u[1,j,k] - u[nx,j,k]) / (solver.x[1] - solver.x[nx])
            end
        end
    end
    # y-direction
    for k in 1:nz, i in 1:nx
        if solver.scheme == :central
            for j in 2:ny-1
                dudy[i,j,k] = (u[i,j+1,k] - u[i,j-1,k]) / (solver.y[j+1] - solver.y[j-1])
            end
            dudy[i,1,k] = (u[i,2,k] - u[i,end,k]) / (solver.y[2] - solver.y[end])
            dudy[i,ny,k] = (u[i,1,k] - u[i,ny-1,k]) / (solver.y[1] - solver.y[ny-1])
        else
            if solver.c[2] >= 0
                for j in 2:ny
                    dudy[i,j,k] = (u[i,j,k] - u[i,j-1,k]) / (solver.y[j] - solver.y[j-1])
                end
                dudy[i,1,k] = (u[i,1,k] - u[i,end,k]) / (solver.y[1] - solver.y[end])
            else
                for j in 1:ny-1
                    dudy[i,j,k] = (u[i,j+1,k] - u[i,j,k]) / (solver.y[j+1] - solver.y[j])
                end
                dudy[i,ny,k] = (u[i,1,k] - u[i,ny,k]) / (solver.y[1] - solver.y[ny])
            end
        end
    end
    # z-direction
    for j in 1:ny, i in 1:nx
        if solver.scheme == :central
            for k in 2:nz-1
                dudz[i,j,k] = (u[i,j,k+1] - u[i,j,k-1]) / (solver.z[k+1] - solver.z[k-1])
            end
            dudz[i,j,1] = (u[i,j,2] - u[i,j,end]) / (solver.z[2] - solver.z[end])
            dudz[i,j,nz] = (u[i,j,1] - u[i,j,nz-1]) / (solver.z[1] - solver.z[nz-1])
        else
            if solver.c[3] >= 0
                for k in 2:nz
                    dudz[i,j,k] = (u[i,j,k] - u[i,j,k-1]) / (solver.z[k] - solver.z[k-1])
                end
                dudz[i,j,1] = (u[i,j,1] - u[i,j,end]) / (solver.z[1] - solver.z[end])
            else
                for k in 1:nz-1
                    dudz[i,j,k] = (u[i,j,k+1] - u[i,j,k]) / (solver.z[k+1] - solver.z[k])
                end
                dudz[i,j,nz] = (u[i,j,1] - u[i,j,nz]) / (solver.z[1] - solver.z[nz])
            end
        end
    end
    return -solver.c[1] .* dudx .- solver.c[2] .* dudy .- solver.c[3] .* dudz
end

function run!(solver::Convection3DSolver)
    nt = length(solver.t)
    for n in 2:nt
        dt = solver.t[n] - solver.t[n-1]
        solver.timestepper(solver.u, u -> rhs!(solver, u), dt)
        if solver.bcs !== nothing
            apply_bcs!(solver.u, solver.x, solver.y, solver.z, solver.bcs)
        end
        solver.write_handler(n, solver.t[n], solver.u)
    end
    closeWriteHandler(solver.write_handler)
    return nothing
end
