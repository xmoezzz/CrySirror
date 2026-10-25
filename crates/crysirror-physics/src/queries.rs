// SPDX-License-Identifier: MPL-2.0
use crate::{
    shapes::*, world::groups, BodyHandle, CharacterConfig, CharacterMove, PhysicsWorld, Pose,
    QueryHit, QueryOptions, Shape as PhysicsShape,
};
use anyhow::{ensure, Result};
use glam::Vec3;
use rapier3d::{
    control::{CharacterAutostep, CharacterLength, KinematicCharacterController},
    prelude::*,
};
impl PhysicsWorld {
    fn query_filter<'a>(
        &self,
        opts: QueryOptions,
        predicate: &'a dyn Fn(ColliderHandle, &Collider) -> bool,
    ) -> Result<QueryFilter<'a>> {
        let mut filter = QueryFilter::default();
        if !opts.include_sensors {
            filter = filter.exclude_sensors();
        }
        if let Some(groups_value) = opts.groups {
            filter = filter.groups(groups(groups_value));
        }
        if let Some(ignore) = opts.ignore {
            filter = filter.exclude_rigid_body(self.native_handle(ignore)?);
        }
        filter.predicate = Some(predicate);
        Ok(filter)
    }
    fn accepts(&self, h: ColliderHandle, opts: QueryOptions) -> bool {
        let Some((body, index)) = self.collider_owner.get(&h) else {
            return false;
        };
        let entry = &self.entries[body];
        entry.desc.enabled
            && entry.collider_desc[*index as usize].enabled
            && (!opts.static_only || entry.desc.kind == crate::BodyKind::Fixed)
            && (!opts.exclude_characters || !entry.desc.character)
    }
    fn hit(&self, h: ColliderHandle, point: Vec3, normal: Vec3, distance: f32) -> QueryHit {
        let &(body, collider) = &self.collider_owner[&h];
        QueryHit {
            body: self.public_handle(body),
            key: self.entries[&body].key,
            collider,
            point,
            normal,
            distance,
        }
    }
    pub fn raycast(
        &mut self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
        opts: QueryOptions,
    ) -> Result<Option<QueryHit>> {
        ensure!(
            origin.is_finite()
                && direction.is_finite()
                && direction.length_squared() > 1e-12
                && max.is_finite()
                && max > 0.,
            "invalid physics ray"
        );
        self.update_queries();
        let direction = direction.normalize();
        let ray = Ray::new(point(origin), vector(direction));
        let predicate = |h, _: &Collider| self.accepts(h, opts);
        let filter = self.query_filter(opts, &predicate)?;
        Ok(self
            .queries
            .cast_ray_and_get_normal(&self.bodies, &self.colliders, &ray, max, true, filter)
            .map(|(h, hit)| {
                self.hit(
                    h,
                    origin + direction * hit.time_of_impact,
                    vec3(hit.normal),
                    hit.time_of_impact,
                )
            }))
    }
    pub fn sphere_cast(
        &mut self,
        origin: Vec3,
        direction: Vec3,
        radius: f32,
        max: f32,
        opts: QueryOptions,
    ) -> Result<Option<QueryHit>> {
        self.cast_shape(
            &PhysicsShape::Ball { radius },
            Pose::at(origin),
            direction,
            max,
            opts,
        )
    }
    pub fn cast_shape(
        &mut self,
        shape: &PhysicsShape,
        position: Pose,
        direction: Vec3,
        max: f32,
        opts: QueryOptions,
    ) -> Result<Option<QueryHit>> {
        validate_pose(position)?;
        ensure!(
            direction.is_finite()
                && direction.length_squared() > 1e-12
                && max.is_finite()
                && max > 0.,
            "invalid shape cast"
        );
        let shared = self.query_shape(shape)?;
        self.update_queries();
        let direction = direction.normalize();
        let predicate = |h, _: &Collider| self.accepts(h, opts);
        let filter = self.query_filter(opts, &predicate)?;
        let options = rapier3d::parry::query::ShapeCastOptions {
            max_time_of_impact: max,
            target_distance: 0.,
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: true,
        };
        Ok(self
            .queries
            .cast_shape(
                &self.bodies,
                &self.colliders,
                &iso(position),
                &vector(direction),
                shared.as_ref(),
                options,
                filter,
            )
            .map(|(h, hit)| {
                self.hit(
                    h,
                    position.translation + direction * hit.time_of_impact,
                    vec3(*hit.normal1),
                    hit.time_of_impact,
                )
            }))
    }
    pub fn overlaps(
        &mut self,
        shape: &PhysicsShape,
        position: Pose,
        opts: QueryOptions,
    ) -> Result<Vec<QueryHit>> {
        validate_pose(position)?;
        let shared = self.query_shape(shape)?;
        self.update_queries();
        let predicate = |h, _: &Collider| self.accepts(h, opts);
        let filter = self.query_filter(opts, &predicate)?;
        let mut out = Vec::new();
        self.queries.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &iso(position),
            shared.as_ref(),
            filter,
            |h| {
                out.push(self.hit(h, position.translation, Vec3::ZERO, 0.));
                true
            },
        );
        out.sort_by_key(|h| (h.key, h.collider));
        Ok(out)
    }
    pub fn move_character(
        &mut self,
        h: BodyHandle,
        translation: Vec3,
        config: CharacterConfig,
    ) -> Result<CharacterMove> {
        ensure!(
            translation.is_finite()
                && [
                    config.offset,
                    config.step_height,
                    config.min_step_width,
                    config.snap_to_ground,
                    config.max_slope_radians,
                    config.mass
                ]
                .iter()
                .all(|v| v.is_finite())
                && config.offset > 0.
                && config.step_height >= 0.
                && config.min_step_width > 0.
                && config.snap_to_ground >= 0.
                && (0.0..std::f32::consts::FRAC_PI_2).contains(&config.max_slope_radians)
                && config.mass > 0.,
            "invalid character motion/configuration"
        );
        let body = self.native_handle(h)?;
        let entry = &self.entries[&body];
        ensure!(
            entry.colliders.len() == 1,
            "character control requires one convex collider"
        );
        ensure!(
            !matches!(
                entry.collider_desc[0].shape,
                PhysicsShape::TriMesh { .. } | PhysicsShape::HeightField { .. }
            ),
            "character needs a convex shape"
        );
        let collider = entry.colliders[0];
        self.update_queries();
        let collider = &self.colliders[collider];
        let shape = collider.shared_shape().clone();
        let pos = *collider.position();
        let groups_value = collider.collision_groups();
        let controller = KinematicCharacterController {
            offset: CharacterLength::Absolute(config.offset),
            slide: config.slide,
            autostep: if config.step_height > 0. {
                Some(CharacterAutostep {
                    max_height: CharacterLength::Absolute(config.step_height),
                    min_width: CharacterLength::Absolute(config.min_step_width),
                    include_dynamic_bodies: config.push_dynamic,
                })
            } else {
                None
            },
            snap_to_ground: if config.snap_to_ground > 0. {
                Some(CharacterLength::Absolute(config.snap_to_ground))
            } else {
                None
            },
            max_slope_climb_angle: config.max_slope_radians,
            min_slope_slide_angle: (config.max_slope_radians + 5f32.to_radians())
                .min(std::f32::consts::FRAC_PI_2),
            ..Default::default()
        };
        let entries = &self.entries;
        let owners = &self.collider_owner;
        let predicate = |h, _: &Collider| {
            owners.get(&h).is_some_and(|(body, index)| {
                entries[body].desc.enabled && entries[body].collider_desc[*index as usize].enabled
            })
        };
        let filter = QueryFilter::default()
            .exclude_rigid_body(body)
            .exclude_sensors()
            .groups(groups_value)
            .predicate(&predicate);
        let mut collisions = Vec::new();
        let dt = self.fixed_dt();
        let movement = controller.move_shape(
            dt,
            &self.bodies,
            &self.colliders,
            &self.queries,
            shape.as_ref(),
            &pos,
            vector(translation),
            filter,
            |event| collisions.push(event),
        );
        if config.push_dynamic {
            controller.solve_character_collision_impulses(
                dt,
                &mut self.bodies,
                &self.colliders,
                &self.queries,
                shape.as_ref(),
                config.mass,
                collisions.iter(),
                filter,
            );
        }
        Ok(CharacterMove {
            translation: vec3(movement.translation),
            grounded: movement.grounded,
        })
    }
}
