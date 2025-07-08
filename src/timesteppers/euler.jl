module TimeSteppersEuler

export euler_step!

function euler_step!(u, rhs, dt)
    # Forward Euler time stepping: u_new = u + dt * rhs(u)
    u .+= dt .* rhs(u)
    return u
end

end
