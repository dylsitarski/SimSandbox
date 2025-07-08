module PostProcessingOutput

export plot_csv_1d, plot_csv_1d_interactive, plot_csv_contour

using CSV
using DataFrames
using GLMakie

function plot_csv_1d(csvfile::String; title=nothing)
    df = CSV.read(csvfile, DataFrame)
    x = parse.(Float64, names(df)[2:end])
    t = df[:, 1]
    u = Matrix(df[:, 2:end])
    fig = Figure()
    ax = isnothing(title) ? Axis(fig[1, 1], xlabel="x", ylabel="u") : Axis(fig[1, 1], xlabel="x", ylabel="u", title=title)
    for i in eachindex(t)
        lines!(ax, x, u[i, :], label="t=$(t[i])")
    end
    axislegend(ax)
    save("solution_plot.png", fig)
end

function plot_csv_1d_interactive(csvfile::String; title=nothing)
    df = CSV.read(csvfile, DataFrame)
    x = parse.(Float64, names(df)[2:end])
    t = df[:, 1]
    u = Matrix(df[:, 2:end])
    fig = Figure()
    ax = isnothing(title) ? Axis(fig[1, 1], xlabel="x", ylabel="u") : Axis(fig[1, 1], xlabel="x", ylabel="u", title=title)
    plt = lines!(ax, x, u[1, :])
    s = Slider(fig[2, 1], range=1:length(t), startvalue=1)
    on(s.value) do i
        plt[1][] = Point2f.(x, u[i, :])
        if isnothing(title)
            ax.title[] = "t = $(t[i])"
        else
            ax.title[] = title * ", t = $(t[i])"
        end
    end
    display(fig)
end

function plot_csv_contour(csvfile::String; title=nothing)
    df = CSV.read(csvfile, DataFrame)
    x = parse.(Float64, names(df)[2:end])
    t = df[:, 1]  # already Float64
    u = Matrix{Float64}(df[:, 2:end])
    fig = Figure()
    ax = isnothing(title) ? Axis(fig[1, 1], xlabel="x", ylabel="t") : Axis(fig[1, 1], xlabel="x", ylabel="y", title=title)
    hm = heatmap!(ax, t, x, u')  # transpose u so x is on x-axis, t is on y-axis
    Colorbar(fig[1, 2], hm)
    save("solution_contour.png", fig)
end

end
