use crate::integrators::Integrator;
use simsandbox_core::block::{Block, BlockSpec, Equations, MethodSpec, StepContext, StepIo};
use simsandbox_core::Result;

/// A model definition in the equation domain: equations plus metadata.
///
/// Outputs must depend on state and parameters only (see
/// [`Equations`](simsandbox_core::block::Equations)).
pub trait OdeSystem: Equations + Send {
    fn spec(&self) -> &BlockSpec;

    /// Initial state from parameter values.
    fn initial_state(&self, p: &[f64], x0: &mut [f64]);

    /// For each rate, the states it depends on: the structure of the
    /// Jacobian, for implicit methods (ADR 0060). `None` if not known;
    /// it is then found by perturbing each state.
    fn sparsity(&self) -> Option<Vec<Vec<usize>>> {
        None
    }
}

/// An [`OdeSystem`] computed by a chosen [`Integrator`]: a kernel Block.
///
/// Inputs are held constant over each communication step (zero-order hold).
pub struct OdeBlock {
    system: Box<dyn OdeSystem>,
    integrator: Integrator,
    method: MethodSpec,
    /// What an implicit method learns of the system's structure.
    workspace: crate::implicit::Workspace,
}

impl OdeBlock {
    pub fn new(system: impl OdeSystem + 'static, integrator: Integrator) -> Self {
        Self::from_box(Box::new(system), integrator)
    }

    pub fn from_box(system: Box<dyn OdeSystem>, integrator: Integrator) -> Self {
        let method = integrator.method_spec();
        Self {
            system,
            integrator,
            method,
            workspace: Default::default(),
        }
    }

    /// Switch method without touching the model definition.
    pub fn set_integrator(&mut self, integrator: Integrator) {
        self.method = integrator.method_spec();
        self.integrator = integrator;
    }
}

impl Block for OdeBlock {
    fn spec(&self) -> &BlockSpec {
        self.system.spec()
    }

    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn init(&mut self, params: &[f64], state: &mut [f64]) -> Result<()> {
        self.system.initial_state(params, state);
        Ok(())
    }

    fn initial_params(&self, state: &[f64], params: &[f64]) -> Option<Vec<f64>> {
        initial_params_by_name(self.spec(), state, params, |p, x| {
            self.system.initial_state(p, x)
        })
    }

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        let sys = self.system.as_ref();
        let f = |t: f64, x: &[f64], dx: &mut [f64]| sys.rhs(t, x, io.inputs, io.params, dx);
        let structure = || sys.sparsity();
        self.integrator.advance_in(
            &f,
            ctx.t,
            ctx.dt,
            io.state,
            &mut self.workspace,
            Some(&structure),
        )?;
        // With the inputs held over the step; the master then makes
        // feedthrough outputs consistent (ADR 0040).
        sys.outputs(ctx.t + ctx.dt, io.state, io.inputs, io.params, io.outputs);
        Ok(())
    }

    fn compute_outputs(&self, t: f64, io: StepIo<'_>) -> Result<()> {
        self.system
            .outputs(t, io.state, io.inputs, io.params, io.outputs);
        Ok(())
    }

    fn as_equations(&self) -> Option<&dyn Equations> {
        Some(self.system.as_ref())
    }
}

/// Parameters whose initial state is `state`, for blocks whose states
/// `s` each start from an initial-condition parameter `s0` (typed
/// equations and the built-in systems): set them, then check that
/// `initial_state` gives `state` exactly (ADR 0042).
pub(crate) fn initial_params_by_name(
    spec: &BlockSpec,
    state: &[f64],
    params: &[f64],
    initial_state: impl Fn(&[f64], &mut [f64]),
) -> Option<Vec<f64>> {
    let mut p = params.to_vec();
    for (s, &v) in spec.states.iter().zip(state) {
        let name = format!("{s}0");
        let k = spec
            .params
            .iter()
            .position(|q| q.initial_condition && q.name == name)?;
        p[k] = v;
    }
    let mut check = vec![0.0; state.len()];
    initial_state(&p, &mut check);
    (check == state).then_some(p)
}
