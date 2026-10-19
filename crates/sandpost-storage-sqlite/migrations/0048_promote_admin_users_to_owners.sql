UPDATE users SET global_role = 'owner' WHERE global_role = 'admin';

CREATE TRIGGER users_reject_admin_insert
BEFORE INSERT ON users
WHEN NEW.global_role = 'admin'
BEGIN
    SELECT RAISE(ABORT, 'admin is a legacy global role');
END;

CREATE TRIGGER users_reject_admin_update
BEFORE UPDATE OF global_role ON users
WHEN NEW.global_role = 'admin'
BEGIN
    SELECT RAISE(ABORT, 'admin is a legacy global role');
END;
