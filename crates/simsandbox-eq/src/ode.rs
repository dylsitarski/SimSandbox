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
}

/// An [`OdeSystem`] computed by a chosen [`Integrator`]: a kernel Block.
///
/// Inputs are held constant over each communication step (zero-order hold).
pub struct OdeBlock {
    system: Box<dyn OdeSystem>,
    integrator: Integrator,
    method: MethodSpec,
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

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        let sys = self.system.as_ref();
        let f = |t: f64, x: &[f64], dx: &mut [f64]| sys.rhs(t, x, io.inputs, io.params, dx);
        self.integrator.advance(&f, ctx.t, ctx.dt, io.state)?;
        sys.outputs(ctx.t + ctx.dt, io.state, io.params, io.outputs);
        Ok(())
    }

    fn compute_outputs(&self, t: f64, io: StepIo<'_>) -> Result<()> {
        self.system.outputs(t, io.state, io.params, io.outputs);
        Ok(())
    }

    fn as_equations(&self) -> Option<&dyn Equations> {
        Some(self.system.as_ref())
    }
}
