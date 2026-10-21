-- GNS3 deletes every ACE below a project's path when the project is deleted.
-- Without this, an ACE or pool membership left behind for a deleted project
-- would grant access to a later project created or imported with the same
-- project_id.
CREATE TRIGGER project_acl_cleanup_after_project_delete
AFTER DELETE ON projects
FOR EACH ROW
BEGIN
    DELETE FROM auth_acl
    WHERE path = '/projects/' || OLD.project_id
       OR substr(path, 1, length('/projects/' || OLD.project_id) + 1) = '/projects/' || OLD.project_id || '/'
       OR (substr(path, 1, 7) = '/pools/'
           AND (substr(path, -length('/resources/' || OLD.project_id)) = '/resources/' || OLD.project_id
                OR instr(path, '/resources/' || OLD.project_id || '/') > 0));
    DELETE FROM resources
    WHERE resource_type = 'project' AND resource_id = OLD.project_id;
END;
