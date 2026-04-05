use serde::Serialize;

/// AST node representing a parsed Gmail search query.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryNode {
    And(Vec<QueryNode>),
    Or(Vec<QueryNode>),
    Not(Box<QueryNode>),
    Group(Box<QueryNode>),
    Operator { key: String, value: String, negated: bool },
    Term(String),
    Quoted(String),
}

/// Error returned when a query fails to parse.
#[derive(Debug, Serialize)]
pub struct QueryError {
    pub error: String,
    pub message: String,
    pub hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
    pub query: String,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for QueryError {}

const MAX_QUERY_LENGTH: usize = 1000;

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),           // plain word
    Quoted(String),         // content inside quotes (quotes stripped)
    Operator(String, String, bool), // key, value, negated — value already unquoted if applicable
    Or,                     // literal "OR"
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
    Negation,               // standalone '-' that prefixes the next token
}

#[derive(Debug, Clone)]
struct PosToken {
    token: Token,
    pos: usize,
}

fn tokenize(input: &str) -> Result<Vec<PosToken>, QueryError> {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut tokens: Vec<PosToken> = Vec::new();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }

        let start = i;

        match chars[i] {
            '(' => {
                tokens.push(PosToken { token: Token::OpenParen, pos: start });
                i += 1;
            }
            ')' => {
                tokens.push(PosToken { token: Token::CloseParen, pos: start });
                i += 1;
            }
            '{' => {
                tokens.push(PosToken { token: Token::OpenBrace, pos: start });
                i += 1;
            }
            '}' => {
                tokens.push(PosToken { token: Token::CloseBrace, pos: start });
                i += 1;
            }
            '"' => {
                // Quoted string
                i += 1; // skip opening quote
                let mut s = String::new();
                while i < len && chars[i] != '"' {
                    s.push(chars[i]);
                    i += 1;
                }
                if i < len {
                    i += 1; // skip closing quote
                }
                tokens.push(PosToken { token: Token::Quoted(s), pos: start });
            }
            '-' => {
                // Negation: '-' followed by non-whitespace content
                if i + 1 < len && !chars[i + 1].is_whitespace() && chars[i + 1] != ')' && chars[i + 1] != '}' {
                    i += 1; // consume the '-'
                    // Now tokenize what follows, but we need to mark it as negated
                    // If it's a quoted string, word, or operator
                    if chars[i] == '"' {
                        // -"quoted"
                        i += 1;
                        let mut s = String::new();
                        while i < len && chars[i] != '"' {
                            s.push(chars[i]);
                            i += 1;
                        }
                        if i < len {
                            i += 1;
                        }
                        // We'll push Negation followed by Quoted, and the parser handles it
                        tokens.push(PosToken { token: Token::Negation, pos: start });
                        tokens.push(PosToken { token: Token::Quoted(s), pos: start + 1 });
                    } else if chars[i] == '(' {
                        // -(group)
                        tokens.push(PosToken { token: Token::Negation, pos: start });
                        // Don't consume '(' here — next iteration handles it
                    } else if chars[i] == '{' {
                        tokens.push(PosToken { token: Token::Negation, pos: start });
                    } else {
                        // -word or -key:value
                        let word_start = i;
                        let mut word = String::new();
                        while i < len && !chars[i].is_whitespace() && chars[i] != '(' && chars[i] != ')' && chars[i] != '{' && chars[i] != '}' {
                            // If we hit a quote as part of operator value like key:"val", handle it
                            if chars[i] == '"' {
                                break;
                            }
                            word.push(chars[i]);
                            i += 1;
                        }
                        // Check if this is an operator (contains ':')
                        if let Some(colon_pos) = word.find(':') {
                            let key = word[..colon_pos].to_string();
                            let val_part = &word[colon_pos + 1..];
                            if val_part.is_empty() {
                                // Check if next char is a quote
                                if i < len && chars[i] == '"' {
                                    i += 1;
                                    let mut quoted_val = String::new();
                                    while i < len && chars[i] != '"' {
                                        quoted_val.push(chars[i]);
                                        i += 1;
                                    }
                                    if i < len {
                                        i += 1;
                                    }
                                    tokens.push(PosToken {
                                        token: Token::Operator(key, quoted_val, true),
                                        pos: start,
                                    });
                                } else {
                                    // Operator with missing value
                                    tokens.push(PosToken {
                                        token: Token::Operator(key, String::new(), true),
                                        pos: start,
                                    });
                                }
                            } else {
                                tokens.push(PosToken {
                                    token: Token::Operator(key, val_part.to_string(), true),
                                    pos: start,
                                });
                            }
                        } else {
                            tokens.push(PosToken { token: Token::Negation, pos: start });
                            tokens.push(PosToken { token: Token::Word(word), pos: word_start });
                        }
                    }
                } else {
                    // Standalone '-' treated as a term
                    tokens.push(PosToken { token: Token::Word("-".into()), pos: start });
                    i += 1;
                }
            }
            _ => {
                // Word or operator
                let mut word = String::new();
                while i < len && !chars[i].is_whitespace() && chars[i] != '(' && chars[i] != ')' && chars[i] != '{' && chars[i] != '}' {
                    if chars[i] == '"' {
                        break;
                    }
                    word.push(chars[i]);
                    i += 1;
                }

                // Check for operator (word contains ':')
                if let Some(colon_pos) = word.find(':') {
                    let key = word[..colon_pos].to_string();
                    let val_part = &word[colon_pos + 1..];
                    if val_part.is_empty() {
                        // Value might be a quoted string right after
                        if i < len && chars[i] == '"' {
                            i += 1;
                            let mut quoted_val = String::new();
                            while i < len && chars[i] != '"' {
                                quoted_val.push(chars[i]);
                                i += 1;
                            }
                            if i < len {
                                i += 1;
                            }
                            tokens.push(PosToken {
                                token: Token::Operator(key, quoted_val, false),
                                pos: start,
                            });
                        } else {
                            // Operator with missing value
                            tokens.push(PosToken {
                                token: Token::Operator(key, String::new(), false),
                                pos: start,
                            });
                        }
                    } else {
                        tokens.push(PosToken {
                            token: Token::Operator(key, val_part.to_string(), false),
                            pos: start,
                        });
                    }
                } else if word == "OR" {
                    tokens.push(PosToken { token: Token::Or, pos: start });
                } else {
                    tokens.push(PosToken { token: Token::Word(word), pos: start });
                }
            }
        }
    }

    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<PosToken>,
    pos: usize,
    input: String,
}

impl Parser {
    fn new(tokens: Vec<PosToken>, input: &str) -> Self {
        Self {
            tokens,
            pos: 0,
            input: input.to_string(),
        }
    }

    fn peek(&self) -> Option<&PosToken> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&PosToken> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn error(&self, message: &str, hint: &str, position: Option<usize>) -> QueryError {
        QueryError {
            error: "query_parse_error".into(),
            message: message.into(),
            hint: hint.into(),
            position,
            query: self.input.clone(),
        }
    }

    /// Parse the full query. Returns the root node.
    fn parse_query(&mut self) -> Result<QueryNode, QueryError> {
        let mut nodes = Vec::new();

        while self.pos < self.tokens.len() {
            // Stop if we hit a closing delimiter — the caller (group parser) handles it
            if let Some(pt) = self.peek() {
                match pt.token {
                    Token::CloseParen | Token::CloseBrace => break,
                    _ => {}
                }
            }
            let node = self.parse_or_expr()?;
            nodes.push(node);
        }

        match nodes.len() {
            0 => Err(self.error("Empty query", "Provide a search query", None)),
            1 => Ok(nodes.remove(0)),
            _ => Ok(QueryNode::And(nodes)),
        }
    }

    /// Parse an expression, then check if it's followed by OR.
    fn parse_or_expr(&mut self) -> Result<QueryNode, QueryError> {
        let left = self.parse_single()?;

        // Check for OR chain
        if self.peek().is_some_and(|pt| pt.token == Token::Or) {
            let mut children = vec![left];
            while self.peek().is_some_and(|pt| pt.token == Token::Or) {
                let or_pos = self.peek().unwrap().pos;
                self.advance(); // consume OR
                // Must have something after OR
                if self.pos >= self.tokens.len() {
                    return Err(self.error(
                        "OR requires expression on both sides",
                        "Add an expression after OR",
                        Some(or_pos),
                    ));
                }
                // Also check for closing delimiters right after OR
                if let Some(pt) = self.peek() {
                    match pt.token {
                        Token::CloseParen | Token::CloseBrace => {
                            return Err(self.error(
                                "OR requires expression on both sides",
                                "Add an expression after OR",
                                Some(or_pos),
                            ));
                        }
                        _ => {}
                    }
                }
                let right = self.parse_single()?;
                children.push(right);
            }
            Ok(QueryNode::Or(children))
        } else {
            Ok(left)
        }
    }

    /// Parse a single expression: group, negation, operator, quoted, or word.
    fn parse_single(&mut self) -> Result<QueryNode, QueryError> {
        let pt = self.peek().ok_or_else(|| {
            self.error("Unexpected end of query", "Complete the query", None)
        })?;

        match pt.token.clone() {
            Token::OpenParen => {
                let open_pos = pt.pos;
                self.advance(); // consume '('
                // Check for empty group
                if self.peek().is_some_and(|pt| pt.token == Token::CloseParen) {
                    return Err(self.error(
                        "Empty group",
                        "Add expressions inside the parentheses",
                        Some(open_pos),
                    ));
                }
                let inner = self.parse_query()?;
                // Expect ')'
                match self.peek() {
                    Some(pt) if pt.token == Token::CloseParen => {
                        self.advance();
                        Ok(QueryNode::Group(Box::new(inner)))
                    }
                    _ => Err(self.error(
                        "Unmatched opening parenthesis",
                        "Add a closing ')'",
                        Some(open_pos),
                    )),
                }
            }
            Token::OpenBrace => {
                let open_pos = pt.pos;
                self.advance();
                if self.peek().is_some_and(|pt| pt.token == Token::CloseBrace) {
                    return Err(self.error(
                        "Empty group",
                        "Add expressions inside the braces",
                        Some(open_pos),
                    ));
                }
                let inner = self.parse_query()?;
                match self.peek() {
                    Some(pt) if pt.token == Token::CloseBrace => {
                        self.advance();
                        Ok(QueryNode::Group(Box::new(inner)))
                    }
                    _ => Err(self.error(
                        "Unmatched opening brace",
                        "Add a closing '}'",
                        Some(open_pos),
                    )),
                }
            }
            Token::CloseParen => {
                let pos = pt.pos;
                Err(self.error(
                    "Unmatched closing parenthesis",
                    "Remove the extra ')' or add a matching '('",
                    Some(pos),
                ))
            }
            Token::CloseBrace => {
                let pos = pt.pos;
                Err(self.error(
                    "Unmatched closing brace",
                    "Remove the extra '}' or add a matching '{'",
                    Some(pos),
                ))
            }
            Token::Negation => {
                self.advance(); // consume negation
                let inner = self.parse_single()?;
                Ok(QueryNode::Not(Box::new(inner)))
            }
            Token::Operator(key, value, negated) => {
                let pos = pt.pos;
                self.advance();
                if value.is_empty() {
                    return Err(self.error(
                        &format!("Operator '{}' requires a value", key),
                        &format!("Use {}:value", key),
                        Some(pos),
                    ));
                }
                Ok(QueryNode::Operator { key, value, negated })
            }
            Token::Quoted(s) => {
                self.advance();
                Ok(QueryNode::Quoted(s))
            }
            Token::Word(w) => {
                self.advance();
                Ok(QueryNode::Term(w))
            }
            Token::Or => {
                let pos = pt.pos;
                Err(self.error(
                    "OR requires expression on both sides",
                    "Add an expression before OR",
                    Some(pos),
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Validate a parsed query AST against security rules.
///
/// Rejects queries that reference blocked labels, use disallowed operators,
/// access drafts/trash/spam, or exceed maximum nesting depth.
pub fn validate_query(
    node: &QueryNode,
    allowed_operators: &[&str],
    blocked_label: &str,
    max_depth: usize,
) -> Result<(), QueryError> {
    validate_recursive(node, allowed_operators, blocked_label, max_depth, 0)
}

fn validate_recursive(
    node: &QueryNode,
    allowed_operators: &[&str],
    blocked_label: &str,
    max_depth: usize,
    depth: usize,
) -> Result<(), QueryError> {
    fn validation_error(message: &str, hint: &str) -> QueryError {
        QueryError {
            error: "query_validation_error".into(),
            message: message.into(),
            hint: hint.into(),
            position: None,
            query: String::new(),
        }
    }

    match node {
        QueryNode::Operator { key, value, .. } => {
            let key_lower = key.to_lowercase();
            let value_lower = value.to_lowercase();

            if key_lower == "label" {
                if value_lower == blocked_label.to_lowercase() {
                    return Err(validation_error(
                        "Query references a label used for security filtering",
                        "Remove the label: operator from your query",
                    ));
                }
                return Err(validation_error(
                    "The label: operator is not allowed for agents",
                    "Remove the label: operator from your query",
                ));
            }

            if key_lower == "is" && value_lower == "draft" {
                return Err(validation_error(
                    "Drafts are not accessible",
                    "Remove is:draft from your query",
                ));
            }

            if key_lower == "in" && matches!(value_lower.as_str(), "anywhere" | "trash" | "spam") {
                return Err(validation_error(
                    &format!("The location '{}' is restricted", value),
                    "Use a different location or remove the in: operator",
                ));
            }

            if !allowed_operators.contains(&key_lower.as_str()) {
                return Err(validation_error(
                    &format!(
                        "Operator '{}' is not supported. Supported operators: {}",
                        key,
                        allowed_operators.join(", ")
                    ),
                    &format!("Use one of: {}", allowed_operators.join(", ")),
                ));
            }

            Ok(())
        }
        QueryNode::Group(inner) => {
            if depth >= max_depth {
                return Err(validation_error(
                    &format!("Query nesting depth exceeds maximum of {}", max_depth),
                    "Simplify the query to reduce nesting",
                ));
            }
            validate_recursive(inner, allowed_operators, blocked_label, max_depth, depth + 1)
        }
        QueryNode::Not(inner) => {
            if depth >= max_depth {
                return Err(validation_error(
                    &format!("Query nesting depth exceeds maximum of {}", max_depth),
                    "Simplify the query to reduce nesting",
                ));
            }
            validate_recursive(inner, allowed_operators, blocked_label, max_depth, depth + 1)
        }
        QueryNode::And(children) | QueryNode::Or(children) => {
            if depth >= max_depth {
                return Err(validation_error(
                    &format!("Query nesting depth exceeds maximum of {}", max_depth),
                    "Simplify the query to reduce nesting",
                ));
            }
            for child in children {
                validate_recursive(child, allowed_operators, blocked_label, max_depth, depth + 1)?;
            }
            Ok(())
        }
        QueryNode::Term(_) | QueryNode::Quoted(_) => Ok(()),
    }
}

/// Serialize a query AST back into a Gmail query string.
pub fn reconstruct_query(node: &QueryNode) -> String {
    match node {
        QueryNode::Term(s) => s.clone(),
        QueryNode::Quoted(s) => format!("\"{}\"", s),
        QueryNode::Operator { key, value, negated } => {
            let prefix = if *negated { "-" } else { "" };
            if value.contains(' ') {
                format!("{}{}:\"{}\"", prefix, key, value)
            } else {
                format!("{}{}:{}", prefix, key, value)
            }
        }
        QueryNode::Not(inner) => format!("-{}", reconstruct_query(inner)),
        QueryNode::Group(inner) => format!("({})", reconstruct_query(inner)),
        QueryNode::And(children) => children
            .iter()
            .map(|c| reconstruct_query(c))
            .collect::<Vec<_>>()
            .join(" "),
        QueryNode::Or(children) => children
            .iter()
            .map(|c| reconstruct_query(c))
            .collect::<Vec<_>>()
            .join(" OR "),
    }
}

/// Reconstruct a query with a label exclusion appended.
///
/// Wraps the user query in a group and adds `-label:{label}` to ensure
/// the blocked label is excluded from results.
pub fn reconstruct_with_label_exclusion(node: &QueryNode, label: &str) -> String {
    format!("({}) -label:{}", reconstruct_query(node), label)
}

/// Parse a Gmail search query string into an AST.
///
/// Returns a `QueryError` if the query is invalid, empty, or exceeds the
/// maximum length. This function is the security-critical entry point:
/// every agent query MUST pass through this parser before reaching Gmail.
pub fn parse_query(input: &str) -> Result<QueryNode, QueryError> {
    // Length check
    if input.len() > MAX_QUERY_LENGTH {
        return Err(QueryError {
            error: "query_parse_error".into(),
            message: "Query exceeds maximum length".into(),
            hint: format!("Keep queries under {} characters", MAX_QUERY_LENGTH),
            position: None,
            query: input.to_string(),
        });
    }

    // Empty / whitespace check
    if input.trim().is_empty() {
        return Err(QueryError {
            error: "query_parse_error".into(),
            message: "Empty query".into(),
            hint: "Provide a search query".into(),
            position: None,
            query: input.to_string(),
        });
    }

    let tokens = tokenize(input)?;
    let mut parser = Parser::new(tokens, input);
    let node = parser.parse_query()?;

    // Check for leftover tokens (shouldn't happen if parse_query consumed all)
    if parser.pos < parser.tokens.len() {
        let pt = &parser.tokens[parser.pos];
        return Err(parser.error(
            "Unexpected token after query",
            "Check query syntax",
            Some(pt.pos),
        ));
    }

    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize("hello world").unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn test_tokenize_operator() {
        let tokens = tokenize("from:alice").unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0].token {
            Token::Operator(k, v, neg) => {
                assert_eq!(k, "from");
                assert_eq!(v, "alice");
                assert!(!neg);
            }
            other => panic!("Expected Operator, got {other:?}"),
        }
    }

    #[test]
    fn test_tokenize_negated_operator() {
        let tokens = tokenize("-from:bob").unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0].token {
            Token::Operator(k, v, neg) => {
                assert_eq!(k, "from");
                assert_eq!(v, "bob");
                assert!(neg);
            }
            other => panic!("Expected negated Operator, got {other:?}"),
        }
    }
}
