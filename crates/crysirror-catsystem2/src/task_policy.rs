// SPDX-License-Identifier: MPL-2.0
//! Original native task-number policy. Reserved roots denote CrySirror-owned
//! input/audio services, not private implementations of either subsystem.
use anyhow::{ensure, Context, Result};
use crysirror_core::image_processing::ImageResource;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Clone, Debug)]
pub struct NativeTaskPolicy {
    slots: BTreeSet<u32>,
    current: u32,
    planes: BTreeMap<u32, NativePlane>,
    buttons: BTreeMap<u32, NativeButton>,
    image_lists: BTreeMap<u32, NativeImageList>,
    fes_tasks: BTreeMap<u32, crate::fes_task::FesTask>,
}
/// Native task ancestry is distinct from render graph ancestry. An empty plane
/// owns an engine group; geometry/paint/transforms belong to that shared Layer.
#[derive(Clone, Debug)]
pub struct NativePlane {
    pub id: u32,
    pub parent: u32,
    pub name: Vec<u8>,
    pub layer: u32,
    pub graph_parent: Option<u32>,
    pub native_rect: [i32; 4],
    pub payload: Option<u32>,
    pub image: Option<NativePlaneImage>,
    /// Native this[35], distinct from the last backend visibility written by
    /// parent message48. A direct child show can override a prior parent hide.
    pub own_enabled: bool,
}
/// Original5DE590 type6 controller. The source creates no draw geometry and
/// no input registration until message89 binds a native input task/rectangle.
#[derive(Clone, Debug)]
pub struct NativeButton {
    pub id: u32,
    pub parent: u32,
    pub name: Vec<u8>,
    pub binding_target: Option<u32>,
    pub rect: [i32; 4],
    pub own_enabled: bool,
    pub relative: bool,
    pub result: u32,
    pub event_mode: u32,
    pub full_canvas_rect: bool,
}
#[derive(Clone, Debug)]
pub struct NativePlaneImage {
    pub resource: Arc<dyn ImageResource>,
    pub path: String,
}
#[derive(Clone, Debug)]
pub struct NativeImagePool {
    pub resource: Arc<dyn ImageResource>,
    pub path: String,
    pub size: [u32; 2],
    pub page_size: [u32; 2],
    pub page_count: u32,
    pub dirty_bounds: Option<[i32; 4]>,
    pub upload_position: Option<[i32; 2]>,
    pub native_entry_keys: BTreeSet<u32>,
}
///5F3790 type15.68E6F0 is an empty bitmap wrapper (68E970/68BB00),
/// not a filesystem scanner. No pixels/manager exist before message279.
#[derive(Clone, Debug)]
pub struct NativeImageList {
    pub id: u32,
    pub parent: u32,
    pub name: Vec<u8>,
    pub manager: Option<NativeImagePool>,
    pub field_120: u32,
    pub field_124: u32,
    pub field_132: u32,
    pub upload_position: Option<[i32; 2]>,
    pub base_plane: Option<u32>,
    pub field_140: u32,
    pub bitmap: Option<Arc<dyn ImageResource>>,
    pub bitmap_field4: u32,
    pub bitmap_field8: u32,
    pub bitmap_field532: u32,
    pub bitmap_name: Vec<u16>,
    pub native_entry_keys: BTreeSet<u32>,
}
impl NativePlane {
    pub fn layer_spec(&self) -> Result<crysirror_scene::layer::LayerSpec> {
        // A logical group has no native pixels. The shared Layer API requires
        // a positive extent; its1x1 transparent extent never substitutes the
        // original empty rectangle in native size/rectangle queries.
        Ok(serde_json::from_value(
            serde_json::json!({"id":self.layer,"parent":null,"rect":[0,0,1,1],"paint":{"kind":"solid","color":[0,0,0,0]},"visible":true,"clip":null}),
        )?)
    }
}
impl NativeTaskPolicy {
    pub fn bootstrap() -> Result<Self> {
        // Reviewed ABI: 5DBE40 constructs BASE=0; 5DD3C0 creates INPUT then
        // SOUND; 5DD580 allocates the first unused number, searching root+tree.
        let mut policy = Self {
            slots: BTreeSet::new(),
            current: 0,
            planes: BTreeMap::new(),
            buttons: BTreeMap::new(),
            image_lists: BTreeMap::new(),
            fes_tasks: BTreeMap::new(),
        };
        let root = policy.allocate()?;
        debug_assert_eq!(root, 0);
        policy.allocate()?;
        policy.allocate()?;
        policy.current = policy.allocate()?;
        Ok(policy)
    }
    fn allocate(&mut self) -> Result<u32> {
        let id = (0..65536)
            .find(|id| !self.slots.contains(id))
            .context("CatSystem2 native task number budget")?;
        self.slots.insert(id);
        Ok(id)
    }
    pub fn current(&self) -> u32 {
        self.current
    }
    pub fn create_fes(&mut self,parent:u32,name:Vec<u8>)->Result<u32> {
        ensure!(parent==self.current,"CatSystem2 unsupported FES constructor parent profile");
        ensure!(name.len()<32&&!name.contains(&0),"CatSystem2 native task name extent");
        let id=self.allocate()?;
        //Native currentMain parameters712 startsNUL; no parameter mutation is
        //mapped. Native66 sends26 after attachment, copying that actual empty
        //string.627E50's earlier125 cannot leave the unattached empty task.
        let task=crate::fes_task::FesTask::new(id,parent,name,Vec::new())?;
        self.fes_tasks.insert(id,task);
        Ok(id)
    }
    pub fn bind_fes_image_list(&mut self,id:u32,image_list:u32)->Result<()> {
        let list=self.image_list(image_list)?;
        let pool=list.manager.as_ref().context("CatSystem2 native FES image-list manager missing")?;
        let [w,h]=pool.resource.size();
        let position=list.upload_position;
        let task=self.fes_tasks.get_mut(&id).context("CatSystem2 target is not a native FES task")?;
        task.image_list=Some(image_list);task.native_rect=[0,0,w as i32,h as i32];
        task.upload_position=position;task.message_parameter=0;
        Ok(())
    }
    ///629850 first tick occurs before the parent KCS resumes from71. Merely
    ///exhausting a host quota or polling a resource does not tick native tasks.
    pub fn initialize_fes_tasks(&mut self,canvas:[u32;2])->Result<Vec<crysirror_scene::input_routes::InputReceiverSpec>> {
        use crysirror_scene::input_routes::InputReceiverSpec;
        ensure!(canvas.iter().all(|v|*v<=100000),"CatSystem2 unsupported native FES full-screen rectangle");
        //5DE8A0 resets existing controller results during the native child
        //phase. Input dispatch has already happened; newly created low buttons
        //below retain their constructor/bind result1 until the following tick.
        for button in self.buttons.values_mut(){button.result=0;}
        let ids=self.fes_tasks.values().filter(|f|f.first_tick).map(|f|f.id).collect::<Vec<_>>();
        let mut receivers=Vec::new();
        for id in ids {
            let task=self.fes_tasks.get(&id).unwrap();
            ensure!(task.parent==self.current&&!task.player_loaded&&!task.paused,"CatSystem2 unsupported native FES tick profile");
            //Main12→root1:629850's ancestor walk produces priority2. FES
            //children/other parents are not admitted by this bounded profile.
            let button=self.create_button(id,b"low button".to_vec())?;
            self.bind_button(button.id,id,[-100000,-100000,100000,100000],button.id,true)?;
            let receiver:InputReceiverSpec=serde_json::from_value(serde_json::json!({"id":button.id,"priority":2,"sources":7,"rect":[-100000,-100000,200000,200000],"coverage":null,"enabled":true,"hover_enabled":true,"drag_enabled":true}))?;
            let task=self.fes_tasks.get_mut(&id).unwrap();task.low_button=Some(button.id);task.first_tick=false;
            receivers.push(receiver);
        }
        Ok(receivers)
    }
    pub fn exists(&self, id: u32) -> bool {
        self.slots.contains(&id)
    }
    pub fn create_plane(&mut self, parent: u32, name: Vec<u8>) -> Result<NativePlane> {
        ensure!(
            self.slots.contains(&parent),
            "CatSystem2 task parent missing"
        );
        ensure!(
            name.len() < 32 && !name.contains(&0),
            "CatSystem2 native task name extent"
        );
        let id = self.allocate()?;
        let plane = NativePlane {
            id,
            parent,
            name,
            layer: 0x43000000 + id * 2,
            graph_parent: None,
            native_rect: [0; 4],
            payload: None,
            image: None,
            own_enabled: true,
        };
        self.planes.insert(id, plane.clone());
        Ok(plane)
    }
    pub fn button(&self, id: u32) -> Result<&NativeButton> {
        self.buttons
            .get(&id)
            .context("CatSystem2 target is not a native click controller")
    }
    pub fn bind_button(
        &mut self,
        id: u32,
        target: u32,
        rect: [i32; 4],
        event_mode: u32,
        full_canvas: bool,
    ) -> Result<()> {
        let b = self
            .buttons
            .get_mut(&id)
            .context("CatSystem2 target is not a native click controller")?;
        ensure!(
            b.binding_target.is_none(),
            "CatSystem2 click controller already bound"
        );
        b.binding_target = Some(target);
        b.rect = rect;
        b.event_mode = event_mode;
        b.full_canvas_rect = full_canvas;
        b.result = 1;
        Ok(())
    }
    pub fn create_button(&mut self, parent: u32, name: Vec<u8>) -> Result<NativeButton> {
        ensure!(
            self.slots.contains(&parent),
            "CatSystem2 task parent missing"
        );
        ensure!(
            name.len() < 32 && !name.contains(&0),
            "CatSystem2 native task name extent"
        );
        let id = self.allocate()?;
        let button = NativeButton {
            id,
            parent,
            name,
            binding_target: None,
            rect: [0; 4],
            own_enabled: true,
            relative: false,
            result: 0,
            event_mode: 0,
            full_canvas_rect: false,
        };
        self.buttons.insert(id, button.clone());
        Ok(button)
    }
    pub fn create_image_list(&mut self, parent: u32, name: Vec<u8>) -> Result<NativeImageList> {
        ensure!(
            self.slots.contains(&parent),
            "CatSystem2 task parent missing"
        );
        ensure!(
            name.len() < 32 && !name.contains(&0),
            "CatSystem2 native task name extent"
        );
        let task = NativeImageList {
            id: self.allocate()?,
            parent,
            name,
            manager: None,
            field_120: 1,
            field_124: 0,
            field_132: 0,
            upload_position: None,
            base_plane: None,
            field_140: 1,
            bitmap: None,
            bitmap_field4: 0,
            bitmap_field8: 0,
            bitmap_field532: 1,
            bitmap_name: Vec::new(),
            native_entry_keys: BTreeSet::new(),
        };
        self.image_lists.insert(task.id, task.clone());
        Ok(task)
    }
    ///644370→283→743E10 only stores upload offsets; it does not move a
    /// plane, compose pixels, mark damage or force presentation. Uninitialised
    /// source fields stay None until this setter is actually called.
    pub fn set_image_upload_position(&mut self, id: u32, position: [i32; 2]) -> Result<()> {
        let task = self
            .image_lists
            .get_mut(&id)
            .context("CatSystem2 target is not native image-list task")?;
        task.upload_position = Some(position);
        if let Some(pool) = &mut task.manager {
            pool.upload_position = Some(position);
        }
        Ok(())
    }
    ///643E10→286 selects original CPU/presentation publication policy. It
    /// does not compose or show pixels; the later native draw operation does.
    pub fn set_image_mode(&mut self, id: u32, value: u32) -> Result<()> {
        let task = self
            .image_lists
            .get_mut(&id)
            .context("CatSystem2 target is not native image-list task")?;
        task.field_120 = value;
        task.field_124 = value;
        if value != 0 {
            task.field_140 = 1;
        }
        Ok(())
    }
    pub fn image_list(&self, id: u32) -> Result<&NativeImageList> {
        self.image_lists
            .get(&id)
            .context("CatSystem2 target is not native image-list task")
    }
    pub fn bind_image_pool(
        &mut self,
        id: u32,
        pool: NativeImagePool,
        base: u32,
        image: NativePlaneImage,
    ) -> Result<()> {
        let task = self
            .image_lists
            .get_mut(&id)
            .context("CatSystem2 target is not native image-list task")?;
        ensure!(
            task.manager.is_none() && task.base_plane.is_none(),
            "CatSystem2 unsupported image-list reinitialization"
        );
        task.manager = Some(pool);
        task.base_plane = Some(base);
        let plane = self
            .planes
            .get_mut(&base)
            .context("CatSystem2 base image plane missing")?;
        plane.payload = Some(plane.layer + 1);
        plane.image = Some(image);
        plane.own_enabled = false;
        Ok(())
    }
    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"fes_tasks":self.fes_tasks.values().map(crate::fes_task::FesTask::diagnostics).collect::<Vec<_>>(),"current":self.current,"allocated":self.slots.len(),"image_lists":self.image_lists.values().map(|t|serde_json::json!({"id":t.id,"task_parent":t.parent,"name":String::from_utf8_lossy(&t.name),"manager":t.manager.as_ref().map(|p|serde_json::json!({"path":p.path,"size":p.size,"owner":p.resource.client().0,"bytes":p.resource.rgba().len(),"page_size":p.page_size,"page_count":p.page_count,"dirty_bounds":p.dirty_bounds,"upload_position":p.upload_position,"native_entries":p.native_entry_keys.len()})),"field_120":t.field_120,"field_124":t.field_124,"field_132":t.field_132,"upload_position":t.upload_position,"base_plane":t.base_plane,"field_140":t.field_140,"empty_bitmap":{"resource":t.bitmap.is_some(),"field4":t.bitmap_field4,"field8":t.bitmap_field8,"field532":t.bitmap_field532,"name_utf16":t.bitmap_name},"native_entry_keys":t.native_entry_keys.len()})).collect::<Vec<_>>(),"buttons":self.buttons.values().map(|b|serde_json::json!({"id":b.id,"task_parent":b.parent,"name":String::from_utf8_lossy(&b.name),"binding_target":b.binding_target,"native_rect":b.rect,"own_enabled":b.own_enabled,"relative":b.relative,"result":b.result,"event_mode":b.event_mode,"full_canvas_rect":b.full_canvas_rect})).collect::<Vec<_>>(),"planes":self.planes.values().map(|p|serde_json::json!({"id":p.id,"task_parent":p.parent,"layer":p.layer,"payload":p.payload,"image":p.image.as_ref().map(|i|serde_json::json!({"path":i.path,"size":i.resource.size(),"owner":i.resource.client().0})),"name":String::from_utf8_lossy(&p.name),"graph_parent":p.graph_parent,"native_rect":p.native_rect})).collect::<Vec<_>>()})
    }
    pub fn plane(&self, id: u32) -> Result<&NativePlane> {
        self.planes
            .get(&id)
            .context("CatSystem2 target is not a native Plane task")
    }
    pub fn set_payload(&mut self, id: u32) -> Result<()> {
        let plane = self
            .planes
            .get_mut(&id)
            .context("CatSystem2 target is not a native Plane task")?;
        plane.payload = Some(plane.layer + 1);
        Ok(())
    }
    /// Original task messages47/48 carry the same visibility argument through
    /// the task tree, rather than inheriting transform/graph-parent state.
    /// Each descendant combines that event with its own native this[35].
    pub fn visibility_messages(&mut self, id: u32, enabled: bool) -> Result<Vec<(u32, bool)>> {
        self.planes
            .get_mut(&id)
            .context("CatSystem2 visibility target is not a native Plane")?
            .own_enabled = enabled;
        let mut affected = BTreeSet::from([id]);
        let mut pending = vec![id];
        let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for plane in self.planes.values() {
            children.entry(plane.parent).or_default().push(plane.id);
        }
        while let Some(parent) = pending.pop() {
            for child in children.get(&parent).into_iter().flatten() {
                if affected.insert(*child) {
                    pending.push(*child);
                }
            }
        }
        Ok(affected
            .into_iter()
            .map(|id| {
                let plane = &self.planes[&id];
                (plane.layer, plane.own_enabled && enabled)
            })
            .collect())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn root_input_audio_and_script_have_native_logical_numbers() {
        let tasks = NativeTaskPolicy::bootstrap().unwrap();
        assert_eq!(tasks.current(), 3);
        assert_eq!(tasks.slots.len(), 4);
    }
    #[test]
    fn fes_attach_waits_for_native_child_tick_and_registers_real_shared_low_button() {
        use crysirror_core::state::ClientId;
        use crysirror_scene::input_routes::{InputRouteCommand,InputRouteEvent,InputRoutePhase,InputRouteSource,InputRouting};
        let mut tasks=NativeTaskPolicy::bootstrap().unwrap();
        let old=tasks.create_button(3,b"existing".to_vec()).unwrap();
        tasks.bind_button(old.id,3,[-100000,-100000,100000,100000],5,true).unwrap();
        let id=tasks.create_fes(3,b"main".to_vec()).unwrap();
        let fes=tasks.fes_tasks.get(&id).unwrap();
        assert_eq!(fes.string_bank_owner,Some(3));
        assert!(fes.parameters.is_empty()&&fes.script_name.is_empty());
        assert!(fes.saved_cursor.is_none()&&fes.input_task.is_none()&&fes.sound_task.is_none());
        assert!(fes.first_tick&&fes.low_button.is_none()&&!fes.player_loaded);
        assert_eq!(tasks.buttons.len(),1);
        let mut routes=InputRouting::default();
        let ready=tasks.initialize_fes_tasks([1280,720]).unwrap();
        assert_eq!(ready.len(),1);
        assert_eq!(tasks.button(old.id).unwrap().result,0);
        let low=ready[0].id;
        let button=tasks.button(low).unwrap();
        assert_eq!((button.parent,button.binding_target,button.event_mode,button.result),(id,Some(id),low,1));
        assert_eq!(ready[0].priority,2);
        routes.apply_batch(ClientId(1),&[InputRouteCommand::Put{receiver:ready[0].clone()}]).unwrap();
        routes.dispatch(ClientId(1),InputRouteEvent{source:InputRouteSource::Pointer,code:16,phase:InputRoutePhase::Press,point:[640,360],origin:None}).unwrap();
        assert_eq!(routes.poll(ClientId(1)).unwrap().unwrap().receiver,low);
        assert!(tasks.initialize_fes_tasks([1280,720]).unwrap().is_empty());
        assert_eq!(tasks.button(low).unwrap().result,0);
        assert_eq!(tasks.buttons.len(),2);
        assert!(tasks.create_fes(old.id,Vec::new()).is_err());
    }
    #[test]
    fn image_list_constructs_source_empty_bitmap_wrapper_and_no_fake_image_manager() {
        let mut p = NativeTaskPolicy::bootstrap().unwrap();
        p.create_plane(3, Vec::new()).unwrap();
        p.create_button(3, Vec::new()).unwrap();
        let t = p.create_image_list(3, b"imglist".to_vec()).unwrap();
        assert_eq!(t.id, 6);
        assert_eq!(
            (t.field_120, t.field_124, t.field_132, t.field_140),
            (1, 0, 0, 1)
        );
        assert!(t.manager.is_none());
        assert_eq!(t.base_plane, None);
        assert_eq!(t.upload_position, None);
        p.set_image_upload_position(t.id, [-2, 3]).unwrap();
        assert_eq!(p.image_list(t.id).unwrap().upload_position, Some([-2, 3]));
        assert!(p.image_list(t.id).unwrap().manager.is_none());
        p.image_lists.get_mut(&t.id).unwrap().field_140 = 0;
        p.set_image_mode(t.id, 0).unwrap();
        let z = p.image_list(t.id).unwrap();
        assert_eq!((z.field_120, z.field_124, z.field_140), (0, 0, 0));
        p.set_image_mode(t.id, 1).unwrap();
        let z = p.image_list(t.id).unwrap();
        assert_eq!((z.field_120, z.field_124, z.field_140), (1, 1, 1));
        assert!(t.bitmap.is_none() && t.bitmap_name.is_empty());
        assert_eq!(
            (t.bitmap_field4, t.bitmap_field8, t.bitmap_field532),
            (0, 0, 1)
        );
        assert!(t.native_entry_keys.is_empty());
        assert!(p.plane(t.id).is_err() && p.button(t.id).is_err());
    }
    #[test]
    fn native_type6_is_an_unbound_input_controller_and_cannot_be_used_as_a_plane() {
        let mut tasks = NativeTaskPolicy::bootstrap().unwrap();
        let button = tasks.create_button(3, b"bg button".to_vec()).unwrap();
        assert_eq!(button.id, 4);
        assert_eq!(button.binding_target, None);
        assert_eq!(button.rect, [0; 4]);
        assert!(button.own_enabled);
        assert!(!button.relative);
        assert_eq!(button.result, 0);
        assert!(tasks.exists(4));
        assert!(tasks.plane(4).is_err());
        assert!(tasks.planes.is_empty());
        assert!(tasks.create_button(65535, Vec::new()).is_err());
        assert_eq!(tasks.slots.len(), 5);
    }
    #[test]
    fn plane_group_keeps_native_empty_rect_and_graph_ancestry_separate() {
        use crysirror_core::state::ClientId;
        use crysirror_scene::layer::{LayerCommand, LayerSystem, SpriteTransform};
        let mut tasks = NativeTaskPolicy::bootstrap().unwrap();
        let plane = tasks.create_plane(3, b"plane".to_vec()).unwrap();
        assert_eq!(plane.id, 4);
        assert_eq!(plane.native_rect, [0; 4]);
        assert_eq!(plane.graph_parent, None);
        let group = plane.layer_spec().unwrap();
        assert_eq!(group.parent, None);
        assert!(group.visible);
        assert_eq!(group.rect, [0., 0., 1., 1.]);
        let mut layers = LayerSystem::default();
        let mut group = group;
        group.rect[0] = 100.;
        group.rect[1] = 50.;
        let child:crysirror_scene::layer::LayerSpec=serde_json::from_value(serde_json::json!({"id":9,"parent":plane.layer,"rect":[3,4,2,2],"paint":{"kind":"solid","color":[255,255,255,255]}})).unwrap();
        layers
            .apply_batch(
                ClientId(1),
                &[
                    LayerCommand::Put { layer: group },
                    LayerCommand::Put { layer: child },
                ],
            )
            .unwrap();
        let frame = layers.frame([1280, 720]).unwrap();
        let child = frame.iter().find(|p| p.id == 9).unwrap();
        assert!(
            (child.quad[0][0] - 103.).abs() < 0.0001 && (child.quad[0][1] - 54.).abs() < 0.0001
        );
        let transform = SpriteTransform {
            rotate: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        layers
            .apply_batch(
                ClientId(1),
                &[LayerCommand::SetTransform {
                    id: plane.layer,
                    transform,
                }],
            )
            .unwrap();
        let frame = layers.frame([1280, 720]).unwrap();
        let child = frame.iter().find(|p| p.id == 9).unwrap();
        assert!((child.quad[0][0] - 96.).abs() < 0.0001 && (child.quad[0][1] - 53.).abs() < 0.0001);
        assert!(tasks.create_plane(65535, b"missing".to_vec()).is_err());
    }
    #[test]
    fn native_visibility_events_are_not_persistent_graph_parent_inheritance() {
        let mut tasks = NativeTaskPolicy::bootstrap().unwrap();
        let a = tasks.create_plane(3, b"a".to_vec()).unwrap();
        let b = tasks.create_plane(a.id, b"b".to_vec()).unwrap();
        let c = tasks.create_plane(b.id, b"c".to_vec()).unwrap();
        assert_eq!(
            tasks.visibility_messages(a.id, false).unwrap(),
            vec![(a.layer, false), (b.layer, false), (c.layer, false)]
        );
        assert_eq!(
            tasks.visibility_messages(b.id, true).unwrap(),
            vec![(b.layer, true), (c.layer, true)]
        );
        tasks.visibility_messages(b.id, false).unwrap();
        assert_eq!(
            tasks.visibility_messages(a.id, true).unwrap(),
            vec![(a.layer, true), (b.layer, false), (c.layer, true)]
        );
        assert_eq!(tasks.plane(b.id).unwrap().graph_parent, None);
    }
}
