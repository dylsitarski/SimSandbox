# ----------------------------
# Mesh utility functions
# ----------------------------

"""
    npoints(m::Mesh)
Number of points in the mesh.
"""
npoints(m::Mesh) = size(m.points, 2)

"""
    ncells(m::Mesh)
Number of cells in the mesh.
"""
ncells(m::Mesh) = length(m.cells)

"""
    cell_nodes(m::Mesh, icell::Int)
Return node index vector for cell `icell`.
"""
cell_nodes(m::Mesh, icell::Int) = m.cells[icell]