//! Player inventory: stackable pickups, weapons, ammo and credits, the 10-slot item belt and the inventory grid.
//! Rules follow Deus Ex's Inventory/DeusExPickup/DeusExPlayer pickup handling: every carried item takes
//! invSlotsX x invSlotsY cells of a 5 x 6 grid (maxInvCols / maxInvRows) and can't be picked up without room.

/// DeusExPlayer.maxInvCols / maxInvRows.
pub const INV_COLS: u32 = 5;
pub const INV_ROWS: u32 = 6;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ItemKind {
    /// A pickup (medkit, lockpick, food, armor...). Stackable ones (bCanHaveMultipleCopies) hold up to
    /// `max_copies` in one entry; `max_copies` 1 means it doesn't stack: each one is its own item and slot.
    Pickup { max_copies: u32 },
    Weapon,
    /// Ammo adds rounds to a pool keyed by its class.
    Ammo { amount: u32 },
    /// Credit chits add to the player's credits.
    Credits { amount: u32 },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ItemDef {
    pub class: String,
    pub name: String,
    pub article: String,
    pub pickup_message: String,
    pub belt_name: String,
    /// Texture path of the belt icon.
    pub icon: Option<String>,
    pub kind: ItemKind,
    /// Grid cells taken (invSlotsX, invSlotsY).
    pub inv_size: (u32, u32),
    /// Inventory-screen icon and its drawn size (largeIcon, largeIconWidth/Height).
    pub large_icon: Option<String>,
    pub large_size: (u32, u32),
    pub description: String,
}

impl Default for ItemDef {
    fn default() -> Self {
        ItemDef {
            class: String::new(),
            name: String::new(),
            article: "a".into(),
            pickup_message: String::new(),
            belt_name: String::new(),
            icon: None,
            kind: ItemKind::Pickup { max_copies: 1 },
            inv_size: (1, 1),
            large_icon: None,
            large_size: (42, 37),
            description: String::new(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub def: ItemDef,
    pub count: u32,
    /// Top-left grid cell (invPosX, invPosY).
    pub pos: Option<(u32, u32)>,
    /// Tells apart several items of one class (non-stacking pickups): 0 for the first, then unique.
    pub uid: u32,
    /// What this one item carries (an augmentation canister's two AddAugs).
    pub data: Vec<String>,
}

impl Item {
    /// A key for this item: its class, or class#uid when there are several of it.
    pub fn key(&self) -> String {
        if self.uid == 0 {
            self.def.class.clone()
        } else {
            format!("{}#{}", self.def.class, self.uid)
        }
    }
}

/// The class part of an item key.
pub fn class_of(key: &str) -> &str {
    key.split('#').next().unwrap_or(key)
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Inventory {
    pub items: Vec<Item>,
    /// Belt slots 0..9 hold indices into `items`.
    pub belt: [Option<usize>; 10],
    pub ammo: Vec<(String, u32)>,
    pub credits: u32,
    /// Last uid handed out (Item::uid).
    pub next_uid: u32,
}

impl Inventory {
    /// Try to take an item. Returns the message to show, or `None` if it could not be taken.
    pub fn pick_up(&mut self, def: &ItemDef, copies: u32) -> Option<String> {
        let copies = copies.max(1);
        match &def.kind {
            ItemKind::Credits { amount } => {
                self.credits += amount;
                Some(format!("{amount} credits added"))
            }
            ItemKind::Ammo { amount } => {
                match self.ammo.iter_mut().find(|(c, _)| *c == def.class) {
                    Some((_, n)) => *n += amount,
                    None => self.ammo.push((def.class.clone(), *amount)),
                }
                Some(format!("{} {} {}", def.pickup_message, def.article, def.name))
            }
            ItemKind::Pickup { max_copies } if *max_copies <= 1 => {
                // Not bCanHaveMultipleCopies: Pickup.HandlePickupQuery lets another one in as its own item.
                if !self.add_new(def, 1) {
                    return Some(no_room(def));
                }
                Some(format!("{} {} {}", def.pickup_message, def.article, def.name))
            }
            ItemKind::Pickup { max_copies } => {
                if let Some(it) = self.items.iter_mut().find(|i| i.def.class == def.class) {
                    if it.count >= *max_copies {
                        return Some(format!("You already have as many {} as you can carry", def.name));
                    }
                    it.count = (it.count + copies).min(*max_copies);
                } else if !self.add_new(def, copies) {
                    return Some(no_room(def));
                }
                Some(format!("{} {} {}", def.pickup_message, def.article, def.name))
            }
            ItemKind::Weapon => {
                if self.items.iter().any(|i| i.def.class == def.class) {
                    // Deus Ex takes the weapon's ammo instead; ammo pools come later.
                    return Some(format!("You already have the {}", def.name));
                }
                if !self.add_new(def, 1) {
                    return Some(no_room(def));
                }
                Some(format!("{} {} {}", def.pickup_message, def.article, def.name))
            }
        }
    }

    /// Use up one copy (DeusExPickup.UseOnce). Returns how many are left; the item goes when none are.
    pub fn use_one(&mut self, class: &str) -> u32 {
        let Some(idx) = self.items.iter().position(|i| i.def.class == class) else { return 0 };
        let it = &mut self.items[idx];
        it.count = it.count.saturating_sub(1);
        if it.count > 0 {
            return it.count;
        }
        self.remove(idx);
        0
    }

    /// Take an item out of the inventory altogether (dropped, used up, destroyed); belt slots follow.
    pub fn remove(&mut self, idx: usize) -> Item {
        let it = self.items.remove(idx);
        for b in self.belt.iter_mut() {
            *b = match *b {
                Some(i) if i == idx => None,
                Some(i) if i > idx => Some(i - 1),
                x => x,
            };
        }
        it
    }

    /// DeusExPlayer.DropItem's bookkeeping: one copy of a stack leaves, anything else leaves whole.
    /// Returns what was dropped.
    pub fn drop_one(&mut self, idx: usize) -> Option<ItemDef> {
        let it = self.items.get_mut(idx)?;
        let def = it.def.clone();
        if matches!(def.kind, ItemKind::Pickup { .. }) && it.count > 1 {
            it.count -= 1;
        } else {
            self.remove(idx);
        }
        Some(def)
    }

    /// Grid occupancy (invSlots), leaving out item `skip` (the one being moved).
    pub fn grid(&self, skip: Option<usize>) -> [[bool; INV_COLS as usize]; INV_ROWS as usize] {
        let mut g = [[false; INV_COLS as usize]; INV_ROWS as usize];
        for (i, it) in self.items.iter().enumerate() {
            if Some(i) == skip {
                continue;
            }
            if let Some((c, r)) = it.pos {
                for y in r..(r + it.def.inv_size.1).min(INV_ROWS) {
                    for x in c..(c + it.def.inv_size.0).min(INV_COLS) {
                        g[y as usize][x as usize] = true;
                    }
                }
            }
        }
        g
    }

    /// IsEmptyItemSlotXY: a w x h item fits with its top-left at (col, row).
    pub fn fits(&self, size: (u32, u32), col: u32, row: u32, skip: Option<usize>) -> bool {
        if col + size.0 > INV_COLS || row + size.1 > INV_ROWS {
            return false;
        }
        let g = self.grid(skip);
        (row..row + size.1).all(|y| (col..col + size.0).all(|x| !g[y as usize][x as usize]))
    }

    /// FindInventorySlotXY: the first free spot, row by row.
    pub fn find_slot(&self, size: (u32, u32), skip: Option<usize>) -> Option<(u32, u32)> {
        (0..INV_ROWS).flat_map(|r| (0..INV_COLS).map(move |c| (c, r))).find(|&(c, r)| self.fits(size, c, r, skip))
    }

    /// PlaceItemInSlot, if the item fits there.
    pub fn move_item(&mut self, idx: usize, col: u32, row: u32) -> bool {
        let Some(size) = self.items.get(idx).map(|i| i.def.inv_size) else { return false };
        if !self.fits(size, col, row, Some(idx)) {
            return false;
        }
        self.items[idx].pos = Some((col, row));
        true
    }

    /// Ours (Enhancement "Inventory Auto-Sort"): repack the grid - weapons first, then the rest, biggest first,
    /// by name - first fit, row by row. Nothing moves if they wouldn't all fit.
    pub fn auto_sort(&mut self) {
        let old: Vec<Option<(u32, u32)>> = self.items.iter().map(|i| i.pos).collect();
        let mut order: Vec<usize> = (0..self.items.len()).filter(|&i| old[i].is_some()).collect();
        order.sort_by(|&a, &b| {
            let (ia, ib) = (&self.items[a], &self.items[b]);
            let w = |i: &Item| if i.def.kind == ItemKind::Weapon { 0 } else { 1 };
            let area = |i: &Item| i.def.inv_size.0 * i.def.inv_size.1;
            (w(ia), std::cmp::Reverse(area(ia)), std::cmp::Reverse(ia.def.inv_size.1), &ia.def.name)
                .cmp(&(w(ib), std::cmp::Reverse(area(ib)), std::cmp::Reverse(ib.def.inv_size.1), &ib.def.name))
        });
        for &i in &order {
            self.items[i].pos = None;
        }
        for &i in &order {
            match self.find_slot(self.items[i].def.inv_size, Some(i)) {
                Some(p) => self.items[i].pos = Some(p),
                None => {
                    for (it, p) in self.items.iter_mut().zip(old) {
                        it.pos = p;
                    }
                    return;
                }
            }
        }
    }

    /// Whether there's room for a new item of this size (FindInventorySlot with bSearchOnly).
    pub fn has_room(&self, def: &ItemDef) -> bool {
        self.find_slot(def.inv_size, None).is_some()
    }

    /// Belt slot holding item `idx`.
    pub fn belt_pos(&self, idx: usize) -> Option<usize> {
        self.belt.iter().position(|b| *b == Some(idx))
    }

    /// PersonaInventoryObjectBelt.AddObject: put an item in a belt slot (taking it out of any other first).
    pub fn belt_assign(&mut self, idx: usize, slot: usize) {
        if slot >= 10 || idx >= self.items.len() {
            return;
        }
        for b in self.belt.iter_mut() {
            if *b == Some(idx) {
                *b = None;
            }
        }
        self.belt[slot] = Some(idx);
    }

    /// SwapObjects between two belt slots.
    pub fn belt_swap(&mut self, a: usize, b: usize) {
        if a < 10 && b < 10 {
            self.belt.swap(a, b);
        }
    }

    pub fn add_new(&mut self, def: &ItemDef, count: u32) -> bool {
        self.add_new_at(def, count).is_some()
    }

    /// As add_new, returning the new item's index.
    pub fn add_new_at(&mut self, def: &ItemDef, count: u32) -> Option<usize> {
        let pos = self.find_slot(def.inv_size, None)?;
        let uid = if self.items.iter().any(|i| i.def.class == def.class) {
            self.next_uid += 1;
            self.next_uid
        } else {
            0
        };
        self.items.push(Item { def: def.clone(), count, pos: Some(pos), uid, data: Vec::new() });
        let idx = self.items.len() - 1;
        // First free belt slot 1..9 (0 is the nano key ring's).
        if let Some(slot) = (1..10).find(|&s| self.belt[s].is_none()) {
            self.belt[slot] = Some(idx);
        }
        Some(idx)
    }
}

/// DeusExPlayer.InventoryFull.
pub fn no_room(def: &ItemDef) -> String {
    format!("You don't have enough room in your inventory to pick up the {}", def.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn medkit() -> ItemDef {
        ItemDef {
            class: "MedKit".into(),
            name: "Medkit".into(),
            article: "a".into(),
            pickup_message: "You found".into(),
            belt_name: "MEDKIT".into(),
            kind: ItemKind::Pickup { max_copies: 15 },
            ..Default::default()
        }
    }

    #[test]
    fn pickups_without_multiple_copies_take_their_own_slots() {
        // AugmentationCannister / BallisticArmor: bCanHaveMultipleCopies False -> each one is a separate item.
        let mut inv = Inventory::default();
        let can = ItemDef { class: "AugmentationCannister".into(), name: "Augmentation Canister".into(), ..Default::default() };
        inv.pick_up(&can, 1);
        inv.pick_up(&can, 1);
        inv.pick_up(&medkit(), 1);
        inv.pick_up(&medkit(), 1);
        assert_eq!(inv.items.iter().filter(|i| i.def.class == "AugmentationCannister").count(), 2);
        assert_eq!(inv.items.iter().filter(|i| i.def.class == "MedKit").count(), 1);
        assert_eq!(inv.items.iter().find(|i| i.def.class == "MedKit").unwrap().count, 2);
        assert_ne!(inv.items[0].pos, inv.items[1].pos);
        assert_ne!(inv.items[0].key(), inv.items[1].key());
        assert_eq!(class_of(&inv.items[1].key()), "AugmentationCannister");
    }

    #[test]
    fn grid_fills_and_refuses_when_full() {
        let mut inv = Inventory::default();
        // A 4 x 2 weapon (GEP gun size) and a 2 x 1 one.
        let big = ItemDef { class: "WeaponGEPGun".into(), name: "GEP Gun".into(), kind: ItemKind::Weapon, inv_size: (4, 2), ..Default::default() };
        inv.pick_up(&big, 1);
        assert_eq!(inv.items[0].pos, Some((0, 0)));
        let small = ItemDef { class: "WeaponPistol".into(), kind: ItemKind::Weapon, inv_size: (2, 1), ..Default::default() };
        inv.pick_up(&small, 1);
        assert_eq!(inv.items[1].pos, Some((0, 2)));
        // Move it, then fill the rest with 1 x 1 items until there's no room.
        assert!(inv.move_item(1, 3, 5));
        assert!(!inv.move_item(1, 4, 5));
        let mut n = 0;
        loop {
            let d = ItemDef { class: format!("Thing{n}"), name: "Thing".into(), ..Default::default() };
            let before = inv.items.len();
            let msg = inv.pick_up(&d, 1).unwrap();
            if inv.items.len() == before {
                assert!(msg.contains("enough room"));
                break;
            }
            n += 1;
        }
        // 30 cells - 8 - 2 = 20 single-cell items.
        assert_eq!(n, 20);
        // Dropping one stack copy keeps the item, dropping the last removes it and fixes the belt.
        inv.belt_assign(0, 7);
        assert_eq!(inv.belt_pos(0), Some(7));
        inv.drop_one(0);
        assert!(inv.items.iter().all(|i| i.def.class != "WeaponGEPGun"));
        assert_eq!(inv.belt[7], None);
    }

    #[test]
    fn stacks_and_uses_belt() {
        let mut inv = Inventory::default();
        assert_eq!(inv.pick_up(&medkit(), 1).unwrap(), "You found a Medkit");
        inv.pick_up(&medkit(), 1);
        assert_eq!(inv.items.len(), 1);
        assert_eq!(inv.items[0].count, 2);
        assert_eq!(inv.belt[1], Some(0));
        let c = ItemDef { class: "Credits".into(), kind: ItemKind::Credits { amount: 100 }, ..medkit() };
        assert_eq!(inv.pick_up(&c, 1).unwrap(), "100 credits added");
        assert_eq!(inv.credits, 100);
    }
}
