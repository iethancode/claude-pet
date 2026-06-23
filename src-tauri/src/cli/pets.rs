// `claude-pet pets` — list available pets.

use crate::domain::pets::list_pets;

pub fn run() -> anyhow::Result<()> {
    let pets = list_pets();
    if pets.is_empty() {
        println!("no pets found");
        return Ok(());
    }
    for pet in pets {
        println!("{}\t{}\t{}x{} frames", pet.id, pet.display_name, pet.columns, pet.rows);
    }
    Ok(())
}
