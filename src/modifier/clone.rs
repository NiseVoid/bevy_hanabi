//! Modifiers to duplicate particles.

use std::hash::{Hash, Hasher};

use bevy::{prelude::*, utils::FloatOrd};
use serde::{Deserialize, Serialize};

use crate::{
    calc_func_id, Attribute, BoxedModifier, EvalContext, ExprError, Modifier, ModifierContext,
    Module, ShaderWriter,
};

/// Duplicates a particle and places it in a group.
///
/// Spawners always spawn particles into group 0, so this is the primary way to
/// place particles into groups other than 0. Typical uses of this modifier are
/// to create trails.
///
/// All attributes are copied to the new particle, with the exception of
/// [`Attribute::AGE`], which is reset to zero.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct CloneModifier {
    /// How many seconds must elapse before the particle will be duplicated.
    ///
    /// If this is zero, particles will be duplicated every frame.
    pub spawn_period: f32,
    /// The group that the new particle will be spawned into.
    pub destination_group: u32,
}

#[typetag::serde]
impl Modifier for CloneModifier {
    fn context(&self) -> ModifierContext {
        ModifierContext::Update
    }

    fn attributes(&self) -> &[Attribute] {
        &[]
    }

    fn boxed_clone(&self) -> BoxedModifier {
        Box::new(*self)
    }

    fn apply(&self, module: &mut Module, context: &mut ShaderWriter) -> Result<(), ExprError> {
        let func_id = calc_func_id(self);
        let func_name = format!("duplicate_{0:016X}", func_id);
        let multiple_count_name = format!("multiple_count_{0:016X}", func_id);

        context.make_fn(
            &func_name,
            "particle: ptr<function, Particle>",
            module,
            &mut |_m: &mut Module, context: &mut dyn EvalContext| -> Result<String, ExprError> {
                let age_reset_code = if context.particle_layout().contains(Attribute::AGE) {
                    format!("particle_buffer.particles[index].{} = 0.0;", Attribute::AGE.name())
                } else {
                    "".to_owned()
                };

                Ok(format!(
                    r##"
                    let base_index = particle_groups[{dest}u].indirect_index;

                    // Recycle a dead particle.
                    let dead_index = atomicSub(&render_group_indirect[{dest}u].dead_count, 1u) - 1u;
                    let index = indirect_buffer.indices[3u * (base_index + dead_index) + 2u];

                    // Copy particle in.
                    particle_buffer.particles[index] = *particle;
                    {age_reset_code}

                    // Mark as alive.
                    atomicAdd(&render_group_indirect[{dest}u].alive_count, 1u);

                    // Add instance.
                    let ping = render_effect_indirect.ping;
                    let indirect_index = atomicAdd(&render_group_indirect[{dest}u].instance_count, 1u);
                    indirect_buffer.indices[3u * (base_index + indirect_index) + ping] = index;
                "##,
                    dest = self.destination_group,
                ))
            },
        )?;

        if self.spawn_period <= 0.0 {
            context.main_code += &format!("{func}(&particle);", func = func_name);
        } else {
            // Calculate the number of multiples of `spawn_period` that fall
            // between the last tick and this one, and spawn one particle for
            // each such multiple.
            //
            // https://stackoverflow.com/a/31871205
            context.main_code += &format!(
                r##"
                let {multiple_count} = max(0, i32(floor({b} / {m})) - i32(ceil(({b} - {delta}) / {m})) + 1);
                for (var i = 0; i < {multiple_count}; i += 1) {{
                    {func}(&particle);
                }}
            "##,
                func = func_name,
                multiple_count = multiple_count_name,
                b = "sim_params.time",
                delta = "sim_params.delta_time",
                m = self.spawn_period
            );
        }

        Ok(())
    }
}

impl CloneModifier {
    /// Creates a new [`CloneModifier`] that will duplicate particles every
    /// `spawn_period` seconds into the `destination_group`.
    pub fn new(spawn_period: f32, destination_group: u32) -> CloneModifier {
        CloneModifier {
            spawn_period,
            destination_group,
        }
    }
}

impl Eq for CloneModifier {}

impl Hash for CloneModifier {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        FloatOrd(self.spawn_period).hash(state);
        self.destination_group.hash(state);
    }
}
