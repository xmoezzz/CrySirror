// SPDX-License-Identifier: MPL-2.0
//! Ren'Py 6.99 execution of sequential ATL placement and displayable steps.
use crate::{
    atl::{RawBlock, RawStatement},
    engine::state,
    lexer::PyExpr,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm};
use crysirror_scene::{animation::ScalarTween, layer::Easing};

#[derive(Clone)]
enum Warper {
    Linear,
    Pause,
    Python(Value),
}

#[derive(Clone)]
enum Step {
    Properties {
        duration_ms: u64,
        warper: Warper,
        values: Vec<(Property, Measure)>,
    },
    Child(Value),
}

#[derive(Clone, Copy)]
enum Property {
    X,
    Y,
    AnchorX,
    AnchorY,
    OffsetX,
    OffsetY,
    Alpha,
    Zoom,
    ZoomX,
    ZoomY,
}

#[derive(Clone, Copy, Default)]
struct Measure {
    value: f32,
    relative: bool,
}

#[derive(Clone, Default)]
pub(crate) struct Program {
    steps: Vec<Step>,
    // None is a straight line. Some(None) is an infinite loop.
    repeat: Option<Option<u64>>,
}

#[derive(Clone)]
pub(crate) struct Instance {
    pub program: Program,
    pub start_ms: u64,
}

pub(crate) struct Pose {
    values: [Measure; 10],
    pub child: Value,
    pub running: bool,
}

impl Pose {
    pub fn new(child: Value) -> Self {
        let mut values = [Measure::default(); 10];
        for index in [
            Property::Alpha,
            Property::Zoom,
            Property::ZoomX,
            Property::ZoomY,
        ] {
            values[index as usize].value = 1.;
        }
        Self {
            values,
            child,
            running: false,
        }
    }

    pub fn layer(&self, width: f32, height: f32, canvas: [i32; 2]) -> ([f32; 4], [f32; 2], u8) {
        let value = |prop: Property, extent: f32| {
            let measure = self.values[prop as usize];
            if measure.relative {
                measure.value * extent
            } else {
                measure.value
            }
        };
        let zoom = value(Property::Zoom, 1.);
        let scale = [
            value(Property::ZoomX, 1.) * zoom,
            value(Property::ZoomY, 1.) * zoom,
        ];
        let x = value(Property::X, canvas[0] as f32) + value(Property::OffsetX, 1.)
            - value(Property::AnchorX, width * scale[0]);
        let y = value(Property::Y, canvas[1] as f32) + value(Property::OffsetY, 1.)
            - value(Property::AnchorY, height * scale[1]);
        let alpha = (255. * value(Property::Alpha, 1.).clamp(0., 1.)) as u8;
        ([x.trunc(), y.trunc(), width, height], scale, alpha)
    }
}

fn number(vm: &Vm, value: &Value) -> PyResult<f64> {
    let value = match value {
        Value::Float(value) => *value,
        Value::Int(value) => *value as f64,
        _ => return Err(vm.type_error("ATL requires a number")),
    };
    if !value.is_finite() || !(value as f32).is_finite() {
        return Err(vm.exc_msg("ValueError", "nonfinite ATL value"));
    }
    Ok(value)
}

fn measure(vm: &Vm, value: &Value, position: bool) -> PyResult<Measure> {
    Ok(Measure {
        value: number(vm, value)? as f32,
        relative: position && matches!(value, Value::Float(_)),
    })
}

fn properties(
    vm: &Vm,
    name: &str,
    value: &Value,
    into: &mut Vec<(Property, Measure)>,
) -> PyResult<()> {
    use Property::*;
    let values: &[(Property, bool)] = match name {
        "xpos" => &[(X, true)],
        "ypos" => &[(Y, true)],
        "xanchor" => &[(AnchorX, true)],
        "yanchor" => &[(AnchorY, true)],
        "xoffset" => &[(OffsetX, false)],
        "yoffset" => &[(OffsetY, false)],
        "xalign" => &[(X, true), (AnchorX, true)],
        "yalign" => &[(Y, true), (AnchorY, true)],
        "xcenter" => {
            into.push((
                AnchorX,
                Measure {
                    value: 0.5,
                    relative: true,
                },
            ));
            &[(X, true)]
        }
        "ycenter" => {
            into.push((
                AnchorY,
                Measure {
                    value: 0.5,
                    relative: true,
                },
            ));
            &[(Y, true)]
        }
        "alpha" => &[(Alpha, false)],
        "zoom" => &[(Zoom, false)],
        "xzoom" => &[(ZoomX, false)],
        "yzoom" => &[(ZoomY, false)],
        "pos" | "align" | "anchor" | "offset" => {
            let Some(crysirror_python::Payload::Tuple(items)) = value.payload() else {
                return Err(vm.type_error("ATL placement requires a pair"));
            };
            let [x, y] = &**items else {
                return Err(vm.type_error("ATL placement requires a pair"));
            };
            let (x_name, y_name) = match name {
                "pos" => ("xpos", "ypos"),
                "align" => ("xalign", "yalign"),
                "anchor" => ("xanchor", "yanchor"),
                _ => ("xoffset", "yoffset"),
            };
            properties(vm, x_name, x, into)?;
            return properties(vm, y_name, y, into);
        }
        _ => return Err(vm.exc_msg("NotImplementedError", format!("ATL property {name}"))),
    };
    for (field, is_position) in values {
        into.push((*field, measure(vm, value, *is_position)?));
    }
    Ok(())
}

fn duration(vm: &Vm, value: &Value) -> PyResult<u64> {
    let seconds = number(vm, value)?;
    if !(0. ..=86400.).contains(&seconds) {
        return Err(vm.exc_msg("ValueError", "ATL duration outside supported range"));
    }
    Ok((seconds * 1000.).round() as u64)
}

struct Compiler<'a> {
    vm: &'a mut Vm,
    globals: ObjRef,
    locals: Option<ObjRef>,
    program: Program,
}

impl Compiler<'_> {
    fn eval(&mut self, expr: &PyExpr) -> PyResult<Value> {
        let code = crate::pycompile::compile(self.vm, expr, "eval", state(self.vm).profile.dialect)
            .map_err(|error| {
                self.vm
                    .syntax_error(crysirror_python::Error::Runtime(error))
            })?;
        self.vm.run_code(
            code,
            self.globals.clone(),
            self.locals.clone().map(Value::Obj),
        )
    }

    fn block(&mut self, block: &RawBlock, depth: usize) -> PyResult<()> {
        if depth >= 48 || self.program.steps.len() > 4096 {
            return Err(self.vm.exc_msg("ValueError", "ATL program limit"));
        }
        for (index, stmt) in block.statements.iter().enumerate() {
            if self.program.steps.len() >= 4096 {
                return Err(self.vm.exc_msg("ValueError", "ATL step limit"));
            }
            match stmt {
                RawStatement::Block(block) => self.block(block, depth + 1)?,
                RawStatement::ContainsExpr { expr, .. } => {
                    let child = self.eval(expr)?;
                    self.program.steps.push(Step::Child(child));
                }
                RawStatement::Repeat { repeats, .. }
                    if depth == 0 && index + 1 == block.statements.len() =>
                {
                    let count = repeats.as_ref().map(|expr| self.eval(expr)).transpose()?;
                    self.program.repeat = Some(match count {
                        None => None,
                        Some(Value::Int(count)) if count > 0 => Some(count as u64),
                        _ => {
                            return Err(self
                                .vm
                                .type_error("ATL repeat requires a positive integer"));
                        }
                    });
                }
                RawStatement::Multipurpose(raw) => {
                    if !raw.splines.is_empty() || raw.revolution.is_some() {
                        return Err(self
                            .vm
                            .exc_msg("NotImplementedError", "ATL spline or revolution"));
                    }
                    if let [(expr, None)] = raw.expressions.as_slice()
                        && raw.properties.is_empty()
                        && raw.warper.is_none()
                        && raw.warp_function.is_none()
                    {
                        let value = self.eval(expr)?;
                        if let Some(atl) = crate::natives::atl_definition(&value) {
                            if atl.parameters.is_some() {
                                return Err(self.vm.exc_msg(
                                    "NotImplementedError",
                                    "unbound ATL transform parameters",
                                ));
                            }
                            let old = std::mem::replace(&mut self.globals, atl.globals);
                            let old_locals = self.locals.take();
                            let result = self.block(&atl.script, depth + 1);
                            self.globals = old;
                            self.locals = old_locals;
                            result?;
                        } else if matches!(value, Value::Int(_) | Value::Float(_)) {
                            let duration_ms = duration(self.vm, &value)?;
                            self.program.steps.push(Step::Properties {
                                duration_ms,
                                warper: Warper::Pause,
                                values: Vec::new(),
                            });
                        } else {
                            self.program.steps.push(Step::Child(value));
                        }
                        continue;
                    }
                    if !raw.expressions.is_empty() {
                        return Err(self.vm.exc_msg(
                            "NotImplementedError",
                            "ATL child transition or interpolation expression",
                        ));
                    }
                    let duration_ms = if let Some(expr) = &raw.duration {
                        let value = self.eval(expr)?;
                        duration(self.vm, &value)?
                    } else {
                        0
                    };
                    let warper = if let Some(expr) = &raw.warp_function {
                        Warper::Python(self.eval(expr)?)
                    } else {
                        match raw.warper.as_deref() {
                            None | Some("pause" | "instant") => Warper::Pause,
                            Some("linear") => Warper::Linear,
                            Some(name) => Warper::Python(
                                state(self.vm)
                                    .warper_functions
                                    .borrow()
                                    .get(name)
                                    .cloned()
                                    .ok_or_else(|| {
                                        self.vm.exc_msg(
                                            "LookupError",
                                            format!("unregistered ATL warper {name}"),
                                        )
                                    })?,
                            ),
                        }
                    };
                    let mut values = Vec::new();
                    for (name, expr) in &raw.properties {
                        let value = self.eval(expr)?;
                        properties(self.vm, name, &value, &mut values)?;
                    }
                    self.program.steps.push(Step::Properties {
                        duration_ms,
                        warper,
                        values,
                    });
                }
                _ => {
                    return Err(self.vm.exc_msg(
                        "NotImplementedError",
                        format!("ATL control statement at {}:{}", block.loc.0, block.loc.1),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Program {
    pub fn compile(vm: &mut Vm, transform: &crate::natives::AtlDefinition) -> PyResult<Self> {
        let mut compiler = Compiler {
            vm,
            globals: transform.globals.clone(),
            locals: transform.locals.clone(),
            program: Self::default(),
        };
        compiler.block(&transform.script, 0)?;
        if compiler.program.repeat.is_some() && compiler.program.duration_ms() == 0 {
            return Err(compiler
                .vm
                .exc_msg("ValueError", "ATL loop has no duration"));
        }
        Ok(compiler.program)
    }

    fn duration_ms(&self) -> u64 {
        self.steps
            .iter()
            .map(|step| match step {
                Step::Properties { duration_ms, .. } => *duration_ms,
                Step::Child(_) => 0,
            })
            .sum()
    }

    pub fn sample(&self, vm: &mut Vm, time: u64, pose: &mut Pose) -> PyResult<()> {
        let duration = self.duration_ms();
        let elapsed = if let Some(count) = self.repeat {
            let terminal = count.is_some_and(|count| time / duration >= count);
            if time >= duration {
                self.sample_pass(vm, duration, pose)?;
            }
            if terminal {
                return Ok(());
            }
            time % duration
        } else {
            time
        };
        self.sample_pass(vm, elapsed, pose)?;
        pose.running |= self.repeat.is_some() || elapsed < duration;
        Ok(())
    }

    fn sample_pass(&self, vm: &mut Vm, mut elapsed: u64, pose: &mut Pose) -> PyResult<()> {
        for step in &self.steps {
            match step {
                Step::Child(child) => pose.child = child.clone(),
                Step::Properties {
                    duration_ms,
                    warper,
                    values,
                } => {
                    if elapsed >= *duration_ms {
                        for (property, target) in values {
                            pose.values[*property as usize] = *target;
                        }
                        elapsed -= duration_ms;
                        continue;
                    }
                    let progress = match warper {
                        Warper::Linear => None,
                        Warper::Pause => break,
                        Warper::Python(_) if values.is_empty() => break,
                        Warper::Python(warper) => {
                            let result = vm.call1(
                                warper,
                                Value::Float(elapsed as f64 / *duration_ms as f64),
                            )?;
                            Some(number(vm, &result)?)
                        }
                    };
                    for (property, target) in values {
                        let value = &mut pose.values[*property as usize];
                        let sample = if let Some(progress) = progress {
                            (f64::from(value.value)
                                + progress * (f64::from(target.value) - f64::from(value.value)))
                                as f32
                        } else {
                            ScalarTween {
                                start: value.value,
                                end: target.value,
                                started_ms: 0,
                                duration_ms: *duration_ms,
                                easing: Easing::Linear,
                            }
                            .sample(elapsed)
                        };
                        value.value = sample;
                        value.relative = target.relative;
                    }
                    break;
                }
            }
        }
        Ok(())
    }
}
