function periodic_bc(u, idxs, idxs2)
    # Enforce periodicity in the x-direction for the given indices
    # idxs: indices along x to set as periodic (e.g., left or right face)
    for i in idxs
        u[i, :, :] .= u[1, :, :]
    end
    return u
end
