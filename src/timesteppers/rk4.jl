module TimeSteppersRK4

export rk4_step!

function rk4_step!(u, rhs, dt)
    # Classic 4th-order Runge-Kutta time stepping
    k1 = rhs(u)
    k2 = rhs(u .+ 0.5dt .* k1)
    k3 = rhs(u .+ 0.5dt .* k2)
    k4 = rhs(u .+ dt .* k3)
    u .+= (dt/6) .* (k1 + 2k2 + 2k3 + k4)
    return u
end

end
