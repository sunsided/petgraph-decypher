//! Indiana Jones example — replicates the decypher crate's example using
//! petgraph-decypher's graph builder and query engine.

use petgraph_decypher::{
    EdgeData, NodeData, PetgraphCypher, PetgraphCypherMut, Value, build_graph_from_cypher,
};

type IndyGraph = petgraph::Graph<NodeData, EdgeData>;

fn main() {
    let mut graph = build_indiana_jones_graph();
    println!("=== Indiana Jones Graph ===");
    println!(
        "Loaded {} nodes and {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    graph
        .cypher_mut(r#"MATCH (r:Role {name: "Indiana Jones"}) SET r.nickname = "Indy""#)
        .expect("failed to update graph via cypher_mut");

    run_query(&graph, "MATCH (p:Person) RETURN p.name AS name");

    run_query(
        &graph,
        "MATCH (p:Person)-[:PLAYS_IN]->(m:Movie) RETURN p.name AS actor, m.name AS movie",
    );

    run_query(
        &graph,
        r#"MATCH (p:Person)-[:PLAYS_IN]->(m:Movie) WHERE m.name = "Last Crusade" RETURN p.name AS actor"#,
    );

    run_query(
        &graph,
        "MATCH (r:Role)-[:SEEKS]->(a:Artifact) RETURN r.name AS role, a.name AS artifact",
    );

    run_query(
        &graph,
        "MATCH (m:Movie) RETURN DISTINCT toUpper(m.name) AS movie ORDER BY movie",
    );

    run_query(
        &graph,
        "MATCH (r:Role) OPTIONAL MATCH (r)-[:SEEKS]->(a:Artifact) RETURN r.name AS role, r.nickname AS nickname, a.name AS artifact ORDER BY role",
    );

    run_query(
        &graph,
        "MATCH (p:Person)-[:PLAYS_IN]->(m:Movie) WITH m, count(p) AS cast WHERE cast > 1 RETURN m.name AS movie, cast ORDER BY cast DESC, movie",
    );

    run_query(
        &graph,
        "MATCH (p:Person)-[:PLAYS_AS]->(:Role)-[:SEEKS]->(a:Artifact)-[:IN_MOVIE]->(m:Movie) RETURN a.name AS artifact, collect(p.name) AS seekers, m AS movie ORDER BY artifact",
    );
}

fn build_indiana_jones_graph() -> IndyGraph {
    build_graph_from_cypher(
        r#"
        // Actors (Person)
        CREATE (indy:Person {name: "Harrison Ford"})
        CREATE (marcus:Person {name: "Denholm Elliott"})
        CREATE (sallah:Person {name: "John Rhys-Davies"})
        CREATE (henry:Person {name: "Sean Connery"})
        CREATE (elsa:Person {name: "Alison Doody"})
        CREATE (donovan:Person {name: "Julian Glover"})
        CREATE (belloq:Person {name: "Paul Freeman"})
        CREATE (mola_ram:Person {name: "Amrish Puri"})
        CREATE (vogel:Person {name: "Michael Byrne"})
        CREATE (spalko:Person {name: "Cate Blanchett"})

        // Movies
        CREATE (raiders:Movie {name: "Raiders of the Lost Ark"})
        CREATE (temple:Movie {name: "Temple of Doom"})
        CREATE (crusade:Movie {name: "Last Crusade"})
        CREATE (crystal:Movie {name: "Crystal Skull"})

        // Artifacts
        CREATE (ark:Artifact {name: "Ark of the Covenant"})
        CREATE (sankara:Artifact {name: "Sankara Stones"})
        CREATE (grail:Artifact {name: "Holy Grail"})

        // Roles (characters played)
        CREATE (role_indy:Role {name: "Indiana Jones"})
        CREATE (role_marcus:Role {name: "Marcus Brody"})
        CREATE (role_sallah:Role {name: "Sallah"})
        CREATE (role_henry:Role {name: "Henry Jones Sr."})
        CREATE (role_elsa:Role {name: "Elsa Schneider"})
        CREATE (role_donovan:Role {name: "Walter Donovan"})
        CREATE (role_belloq:Role {name: "René Belloq"})
        CREATE (role_mola_ram:Role {name: "Mola Ram"})
        CREATE (role_vogel:Role {name: "Colonel Ernst Vogel"})
        CREATE (role_spalko:Role {name: "Irina Spalko"})

        // Person -> PLAYS_IN -> Movie
        CREATE (indy)-[:PLAYS_IN]->(raiders)
        CREATE (indy)-[:PLAYS_IN]->(temple)
        CREATE (indy)-[:PLAYS_IN]->(crusade)
        CREATE (indy)-[:PLAYS_IN]->(crystal)

        CREATE (marcus)-[:PLAYS_IN]->(raiders)
        CREATE (marcus)-[:PLAYS_IN]->(crusade)

        CREATE (sallah)-[:PLAYS_IN]->(raiders)
        CREATE (sallah)-[:PLAYS_IN]->(crusade)

        CREATE (henry)-[:PLAYS_IN]->(crusade)
        CREATE (elsa)-[:PLAYS_IN]->(crusade)
        CREATE (donovan)-[:PLAYS_IN]->(crusade)

        CREATE (belloq)-[:PLAYS_IN]->(raiders)
        CREATE (mola_ram)-[:PLAYS_IN]->(temple)
        CREATE (vogel)-[:PLAYS_IN]->(crusade)
        CREATE (spalko)-[:PLAYS_IN]->(crystal)

        // Person -> PLAYS_AS -> Role
        CREATE (indy)-[:PLAYS_AS]->(role_indy)
        CREATE (marcus)-[:PLAYS_AS]->(role_marcus)
        CREATE (sallah)-[:PLAYS_AS]->(role_sallah)
        CREATE (henry)-[:PLAYS_AS]->(role_henry)
        CREATE (elsa)-[:PLAYS_AS]->(role_elsa)
        CREATE (donovan)-[:PLAYS_AS]->(role_donovan)
        CREATE (belloq)-[:PLAYS_AS]->(role_belloq)
        CREATE (mola_ram)-[:PLAYS_AS]->(role_mola_ram)
        CREATE (vogel)-[:PLAYS_AS]->(role_vogel)
        CREATE (spalko)-[:PLAYS_AS]->(role_spalko)

        // Role -> SEEKS -> Artifact
        CREATE (role_indy)-[:SEEKS]->(ark)
        CREATE (role_indy)-[:SEEKS]->(sankara)
        CREATE (role_indy)-[:SEEKS]->(grail)

        CREATE (role_belloq)-[:SEEKS]->(ark)
        CREATE (role_mola_ram)-[:SEEKS]->(sankara)
        CREATE (role_vogel)-[:SEEKS]->(grail)

        // Artifact -> IN_MOVIE -> Movie
        CREATE (ark)-[:IN_MOVIE]->(raiders)
        CREATE (sankara)-[:IN_MOVIE]->(temple)
        CREATE (grail)-[:IN_MOVIE]->(crusade)
        "#,
    )
    .expect("failed to build graph")
}

fn run_query(graph: &IndyGraph, query: &str) {
    println!("\n--- Query: {} ---", query);
    match graph.cypher(query) {
        Ok(result) => {
            let rows: Vec<Vec<String>> = result
                .rows()
                .map(|row| row.values().iter().map(|v| render(graph, v)).collect())
                .collect();
            print_table(result.columns(), &rows);
        }
        Err(err) => {
            eprintln!("Query error: {}", err);
        }
    }
}

/// Renders a result value; nodes and relationships are looked up in the graph.
fn render(graph: &IndyGraph, value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "NULL".to_string(),
        Value::List(items) => {
            let items: Vec<String> = items.iter().map(|v| render(graph, v)).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Node(n) => format!(
            "({}{})",
            graph[*n]
                .labels
                .iter()
                .map(|l| format!(":{l}"))
                .collect::<String>(),
            graph[*n]
                .properties
                .get("name")
                .map(|name| format!(" {}", render(graph, name)))
                .unwrap_or_default()
        ),
        Value::Relationship(e) => format!("[:{}]", graph[*e].rel_type),
        other => other.to_string(),
    }
}

fn print_table(headers: &[String], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let hline = "+".to_string()
        + &widths
            .iter()
            .map(|w| "-".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("+")
        + "+";

    println!("{}", hline);
    print!("|");
    for (i, h) in headers.iter().enumerate() {
        print!(" {:width$} |", h, width = widths[i]);
    }
    println!();
    println!("{}", hline);
    for row in rows {
        print!("|");
        for (i, cell) in row.iter().enumerate() {
            print!(" {:width$} |", cell, width = widths[i]);
        }
        println!();
    }
    println!("{}", hline);
}
