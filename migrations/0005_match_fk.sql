-- Round 7: matches table gets the foreign keys the other tables had
-- from the start (orphan matches were possible by direct SQL).

ALTER TABLE matches
  ADD CONSTRAINT fk_matches_taker FOREIGN KEY (taker_id) REFERENCES users(id),
  ADD CONSTRAINT fk_matches_given FOREIGN KEY (given_id) REFERENCES projects(id),
  ADD CONSTRAINT fk_matches_taken FOREIGN KEY (taken_id) REFERENCES projects(id);
